//! Bundles for air-gapped machines (ADR 0122 rule 33): `una ita bundle <file>` writes the
//! lock's registry packages with their provenance into one file, and `una ita unbundle <file>`
//! verifies all of it against the package's own `ita.una` and `ita.lock` before it fills
//! Itaoca.
//!
//! A bundle has the layout of an archive (rule 29) under its own first line:
//!
//! ```text
//! ita-bundle/1
//! <length of the listing>
//! <listing>                    path NUL length NUL hex SHA-256, one line per entry, sorted
//! <each entry's bytes, in listing order>
//! ```
//!
//! Its entries, each a path of rule 27's form:
//!
//! - `packages/<hex>`: the archive of the package of content digest `sha256:<hex>`;
//! - `provenance/<source>/<hex>/checkpoint-<size>`: that package's provenance at the resolution
//!   point of `<size>` ([`crate::provenance`]): the root chain, the checkpoint with its
//!   cosignatures, the release statement and its inclusion proof;
//! - `registry/<source>/<path>`: every file of the source's registry that the client read to
//!   verify the lock's releases of it at the lock's resolution point ([`Recording`]): the root
//!   chain, the checkpoint of that point and the kept checkpoints that time its entries, the
//!   log's tiles and entry bundles, and the name map's tiles of each name and owner, so the
//!   per-name state, the owners' keys and every inclusion proof travel with it. Of each kept
//!   checkpoint, whose cosignatures the client merges across places, every place's distinct
//!   copy travels: the first at its path, the others at `copies/<n>/<path>` ([`copy_path`]),
//!   so unbundling merges the same cosignatures offline and computes the same witness times.
//!
//! **Nothing in a bundle is trusted.** Unbundling reads the bundle once into memory, checks its
//! listing and every entry's hash, and then, for every registry package of the lock, verifies
//! its archive's digest against the lock and its provenance as every load does (rule 17); and
//! opens each source's copy ([`Memory`]) with the registry client at the lock's resolution point
//! (rules 35 to 39): the root chain from the pin of `ita.una`, the checkpoint's log signature and
//! witnesses, the consistency with the checkpoint Itaoca stores for the source and with the
//! lock's own point, the root version stored as a floor, the name's whole
//! chain with its yanks and advisories, and the owner's keys. The release the client reads MUST
//! be the lock's: its log index and digest. Only when every package verifies is anything
//! written; then the source's checkpoint and root version are stored as its floors, through
//! Itaoca's floor lock, so neither ever moves backwards. No
//! clock freshness is asked: the lock's digests and the proofs decide.
//!
//! **A bundle is a place.** Its listing commits to the entries it holds, not to the registry
//! files it should hold, so a registry file it lacks is an omission, as any place's is: the
//! copy is read under [`Fallback`], with the places of the source the caller gives behind it,
//! and a document that none of them serves leaves the state unavailable
//! ([`Failure::Unavailable`], U7013), never a refusal and never a different result.
//!
//! Costs: O(B) for a bundle of B bytes, read and hashed once, with memory O(B); O(e log e) to
//! sort e entries; and per package the provenance's O(log L) hashes.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use sha2::{Digest, Sha256};

use super::{decimal, digest_hex, hex};
use crate::client::{Floors, Omission, OpenError, Point, Registry, shown};
use crate::content::check_path;
use crate::itaoca::{Itaoca, registry_id};
use crate::provenance::{self, Expected};
use crate::transport::{Fallback, Refusal, Transport, segments};
use crate::tree::VerifiedTree;
use crate::vendor::{Problem, check_source};

/// The first line of a bundle.
pub const FORMAT: &str = "ita-bundle/1";
/// The most entries of a bundle.
pub const MAX_ENTRIES: usize = 100_000;
/// The most bytes of a bundle.
pub const MAX_BYTES: u64 = 4 << 30;

/// The path of the archive of the package of `digest`.
pub fn package_path(digest: &str) -> Option<String> {
    digest_hex(digest).map(|hex| format!("packages/{hex}"))
}

/// The path of the provenance of the package of `digest` from `source` at the resolution point
/// of `size`.
pub fn provenance_path(source: &str, digest: &str, size: u64) -> Option<String> {
    check_source(source).ok()?;
    digest_hex(digest).map(|hex| format!("provenance/{source}/{hex}/checkpoint-{size}"))
}

/// A transport that records every file it serves, so a bundle carries exactly what the client
/// read to verify the lock.
pub struct Recording<'t> {
    /// The transport read through.
    pub inner: &'t dyn Transport,
    /// The files served, by path.
    pub seen: RefCell<BTreeMap<String, Vec<u8>>>,
}

impl<'t> Recording<'t> {
    /// A recording of `inner`.
    pub fn new(inner: &'t dyn Transport) -> Recording<'t> {
        Recording {
            inner,
            seen: RefCell::new(BTreeMap::new()),
        }
    }
}

/// The most copies of one file a bundle carries: one per place of a source, its 64 mirrors and
/// its own URL.
pub const MAX_COPIES: usize = 65;

/// The path at which a bundle's copy of a source carries the `n`-th other copy of `path`, `n`
/// from 1: `copies/<n>/<path>`.
pub fn copy_path(n: usize, path: &str) -> String {
    format!("copies/{n}/{path}")
}

impl Transport for Recording<'_> {
    fn get(&self, path: &str, limit: u64) -> Result<Option<Vec<u8>>, Refusal> {
        let got = self.inner.get(path, limit)?;
        if let Some(bytes) = &got {
            self.seen
                .borrow_mut()
                .insert(path.to_owned(), bytes.clone());
        }
        Ok(got)
    }

    /// Every place's copy, each distinct one recorded: the first at `path`, the others at
    /// [`copy_path`], so a bundle carries every copy whose cosignatures the client merged.
    /// O(C^2) for C copies, C at most [`MAX_COPIES`].
    fn get_all(&self, path: &str, limit: u64) -> Result<Vec<Vec<u8>>, Refusal> {
        let copies = self.inner.get_all(path, limit)?;
        let mut distinct: Vec<&Vec<u8>> = Vec::new();
        for bytes in &copies {
            if !distinct.contains(&bytes) && distinct.len() < MAX_COPIES {
                distinct.push(bytes);
            }
        }
        let mut seen = self.seen.borrow_mut();
        for (n, bytes) in distinct.into_iter().enumerate() {
            let at = if n == 0 {
                path.to_owned()
            } else {
                copy_path(n, path)
            };
            seen.insert(at, bytes.clone());
        }
        Ok(copies)
    }

    fn describe(&self) -> String {
        self.inner.describe()
    }
}

/// A source's registry files as a bundle carries them, served from memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    /// The source.
    pub source: String,
    /// The files, by path.
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Transport for Memory {
    fn get(&self, path: &str, limit: u64) -> Result<Option<Vec<u8>>, Refusal> {
        segments(path)?;
        match self.files.get(path) {
            Some(bytes) if bytes.len() as u64 > limit => Err(format!(
                "`{path}` of the bundle's copy of `{}` holds more than {limit} bytes",
                self.source
            )),
            Some(bytes) => Ok(Some(bytes.clone())),
            None => Ok(None),
        }
    }

    /// The copy at `path`, then the other copies the bundle carries for it, at [`copy_path`]
    /// from 1 while there is one: every place's copy the bundle's writer recorded.
    fn get_all(&self, path: &str, limit: u64) -> Result<Vec<Vec<u8>>, Refusal> {
        let mut copies: Vec<Vec<u8>> = self.get(path, limit)?.into_iter().collect();
        if copies.is_empty() {
            return Ok(copies);
        }
        for n in 1..MAX_COPIES {
            match self.get(&copy_path(n, path), limit)? {
                Some(bytes) => copies.push(bytes),
                None => break,
            }
        }
        Ok(copies)
    }

    fn describe(&self) -> String {
        format!("the bundle's copy of `{}`", self.source)
    }
}

/// The copy of `source`'s registry files that the bundle `bytes`, as `read` read it, holds.
pub fn memory(bytes: &[u8], read: &Read, source: &str) -> Memory {
    let prefix = format!("registry/{source}/");
    let files = read
        .entries
        .range(prefix.clone()..)
        .take_while(|(path, _)| path.starts_with(&prefix))
        .filter_map(|(path, range)| {
            Some((
                path.get(prefix.len()..)?.to_owned(),
                bytes.get(range.clone())?.to_vec(),
            ))
        })
        .collect();
    Memory {
        source: source.to_owned(),
        files,
    }
}

/// Whether `path` is an entry a bundle may hold, by its form: rule 27's, and one of the three
/// kinds this module names.
fn is_entry(path: &str) -> bool {
    if check_path(path).is_err() {
        return false;
    }
    let parts: Vec<&str> = path.split('/').collect();
    fn is_hex(text: &str) -> bool {
        text.len() == 64
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
    match parts.as_slice() {
        ["packages", hex] => is_hex(hex),
        ["provenance", source, hex, file] => {
            check_source(source).is_ok()
                && is_hex(hex)
                && file
                    .strip_prefix("checkpoint-")
                    .and_then(decimal)
                    .is_some()
        }
        ["registry", source, rest @ ..] => {
            check_source(source).is_ok() && !rest.is_empty() && segments(&rest.join("/")).is_ok()
        }
        _ => false,
    }
}

/// Writes a bundle of `entries`, paths and bytes. Refused for a path this module does not name,
/// a path given twice, and the bounds.
pub fn write(mut entries: Vec<(String, Vec<u8>)>) -> Result<Vec<u8>, String> {
    if entries.len() > MAX_ENTRIES {
        return Err(format!("a bundle holds at most {MAX_ENTRIES} entries"));
    }
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut listing = String::new();
    let mut total: u64 = 0;
    let mut previous: Option<&str> = None;
    for (path, bytes) in &entries {
        if !is_entry(path) {
            return Err(format!("`{path}` is not an entry of a bundle"));
        }
        if previous == Some(path.as_str()) {
            return Err(format!("`{path}` is given twice"));
        }
        previous = Some(path.as_str());
        total = total.saturating_add(bytes.len() as u64);
        listing.push_str(&format!(
            "{path}\0{}\0{}\n",
            bytes.len(),
            hex(&Sha256::digest(bytes))
        ));
    }
    if total.saturating_add(listing.len() as u64) > MAX_BYTES {
        return Err(format!("a bundle holds at most {MAX_BYTES} bytes"));
    }
    let mut out = Vec::new();
    out.extend_from_slice(FORMAT.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(listing.len().to_string().as_bytes());
    out.push(b'\n');
    out.extend_from_slice(listing.as_bytes());
    for (_, bytes) in &entries {
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

/// A bundle read: each entry's path and the range of its bytes in the bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    /// The entries, by path.
    pub entries: BTreeMap<String, Range<usize>>,
}

/// The bytes up to the next line feed, which is consumed.
fn next_line<'b>(bytes: &'b [u8], at: &mut usize) -> Option<&'b str> {
    let rest = bytes.get(*at..)?;
    let newline = rest.iter().position(|b| *b == b'\n')?;
    let line = rest.get(..newline)?;
    *at += newline + 1;
    std::str::from_utf8(line).ok()
}

/// Reads a bundle from its bytes, read once into memory: its first line, the listing's length
/// and the listing, whose paths are entries of this module in strictly ascending byte order;
/// then each entry's bytes, hashed and compared with the listing; and nothing after.
pub fn read(bytes: &[u8]) -> Result<Read, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("the bundle holds more than {MAX_BYTES} bytes"));
    }
    let mut at = 0;
    if next_line(bytes, &mut at) != Some(FORMAT) {
        return Err(format!("the file does not start with `{FORMAT}`"));
    }
    let length = next_line(bytes, &mut at)
        .and_then(decimal)
        .and_then(|length| usize::try_from(length).ok())
        .ok_or("the bundle's listing length is not a decimal without a leading zero")?;
    let end = at
        .checked_add(length)
        .filter(|end| *end <= bytes.len())
        .ok_or("the bundle's listing runs past its end")?;
    let listing = std::str::from_utf8(bytes.get(at..end).unwrap_or_default())
        .map_err(|_| "the bundle's listing is not UTF-8".to_owned())?;
    if !listing.is_empty() && !listing.ends_with('\n') {
        return Err("the bundle's listing does not end with a line feed".to_owned());
    }
    let mut entries = BTreeMap::new();
    let mut cursor = end;
    let mut previous: Option<&str> = None;
    for line in listing.lines() {
        if entries.len() == MAX_ENTRIES {
            return Err(format!("the bundle holds more than {MAX_ENTRIES} entries"));
        }
        let mut parts = line.split('\0');
        let (Some(path), Some(len), Some(hash), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(
                "an entry of the bundle's listing is not `path NUL length NUL hash`".to_owned(),
            );
        };
        if !is_entry(path) {
            return Err(format!("`{path}` is not an entry of a bundle"));
        }
        if previous.is_some_and(|p| p.as_bytes() >= path.as_bytes()) {
            return Err(format!("the entry `{path}` is out of order or repeated"));
        }
        previous = Some(path);
        let len = decimal(len)
            .and_then(|len| usize::try_from(len).ok())
            .ok_or_else(|| format!("the length of `{path}` is not a decimal"))?;
        let stop = cursor
            .checked_add(len)
            .filter(|stop| *stop <= bytes.len())
            .ok_or_else(|| format!("the length of `{path}` runs past the end of the bundle"))?;
        let contents = bytes.get(cursor..stop).unwrap_or_default();
        if hex(&Sha256::digest(contents)) != hash {
            return Err(format!(
                "the bytes of `{path}` do not have the hash the bundle's listing gives"
            ));
        }
        entries.insert(path.to_owned(), cursor..stop);
        cursor = stop;
    }
    if cursor != bytes.len() {
        return Err("the bundle has bytes after its last entry".to_owned());
    }
    Ok(Read { entries })
}

/// What a bundle holds of one package: its archive and its provenance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packed {
    /// The archive of rule 29.
    pub archive: Vec<u8>,
    /// The provenance document.
    pub provenance: Vec<u8>,
}

/// The entries of a bundle for `packages`, each the package `expected` names, verified as it was
/// loaded, with its archive and provenance; and `registries`, each source's files that the
/// client read ([`Recording`]), below `registry/<source>/`. Refused for a path this module does
/// not name.
pub fn entries(
    packages: &[(Expected, Packed)],
    registries: &[(String, BTreeMap<String, Vec<u8>>)],
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::with_capacity(packages.len() * 2);
    for (expected, packed) in packages {
        let archive = package_path(&expected.digest)
            .ok_or_else(|| format!("`{}` is not a digest", expected.digest))?;
        let provenance =
            provenance_path(&expected.source, &expected.digest, expected.checkpoint_size)
                .ok_or_else(|| format!("`{}` is not a source", expected.source))?;
        out.push((archive, packed.archive.clone()));
        out.push((provenance, packed.provenance.clone()));
    }
    for (source, files) in registries {
        check_source(source)?;
        for (path, bytes) in files {
            out.push((format!("registry/{source}/{path}"), bytes.clone()));
        }
    }
    // Two packages of one digest, source and resolution point are one entry.
    out.sort();
    out.dedup();
    Ok(out)
}

/// Verifies the package `expected` names from the bundle `bytes` that `read` read: its archive's
/// digest is the lock's, and its provenance verifies against the lock and the archive's
/// `ita.una` (rule 17). Returns the archive and the provenance, verified; a package the bundle
/// does not hold is [`Problem::Absent`], any other failure [`Problem::Refused`].
pub fn verify_package(bytes: &[u8], read: &Read, expected: &Expected) -> Result<Packed, Problem> {
    verify_held(bytes, read, expected).map_err(|(absent, why)| {
        if absent {
            Problem::Absent(why)
        } else {
            Problem::Refused(why)
        }
    })
}

fn verify_held(bytes: &[u8], read: &Read, expected: &Expected) -> Result<Packed, (bool, String)> {
    let what = format!("`{} {}`", expected.name, expected.version);
    let refused = |why: String| (false, why);
    let archive_path = package_path(&expected.digest)
        .ok_or_else(|| refused(format!("the digest of {what} is not a digest")))?;
    let provenance_path =
        provenance_path(&expected.source, &expected.digest, expected.checkpoint_size)
            .ok_or_else(|| refused(format!("the source of {what} is not a source")))?;
    let archive = read
        .entries
        .get(&archive_path)
        .and_then(|range| bytes.get(range.clone()))
        .ok_or_else(|| (true, format!("the bundle does not hold {what}")))?;
    let provenance = read
        .entries
        .get(&provenance_path)
        .and_then(|range| bytes.get(range.clone()))
        .ok_or_else(|| refused(format!("the bundle holds {what} without its provenance")))?;
    // What the bundle served is quoted as a message may show it.
    verify_parts(archive, provenance, &what, expected).map_err(refused)
}

fn verify_parts(
    archive: &[u8],
    provenance: &[u8],
    what: &str,
    expected: &Expected,
) -> Result<Packed, String> {
    let tree = VerifiedTree::from_archive(archive).map_err(|refusal| {
        format!("the archive of {what} in the bundle: {}", shown(&refusal))
    })?;
    if tree.digest() != expected.digest {
        return Err(format!(
            "the archive of {what} in the bundle has the digest {}, not the lock's {}",
            tree.digest(),
            expected.digest
        ));
    }
    let document = provenance::parse(provenance).map_err(|refusal| {
        format!("the provenance of {what} in the bundle: {}", shown(&refusal))
    })?;
    let statement = provenance::verify(&document, expected).map_err(|refusal| {
        format!("the provenance of {what} in the bundle: {}", shown(&refusal))
    })?;
    let manifest = tree
        .file("ita.una")
        .ok_or_else(|| format!("{what} in the bundle has no ita.una"))?;
    if statement.manifest != provenance::sha256_digest(manifest) {
        return Err(format!(
            "the ita.una of {what} in the bundle is not the one its release statement names"
        ));
    }
    Ok(Packed {
        archive: archive.to_vec(),
        provenance: provenance.to_vec(),
    })
}

/// Why unbundling stopped, as the command codes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// A package the bundle does not hold (U7011).
    Absent(String),
    /// What the bundle, or a place behind it, holds does not verify (U7010).
    Refused(String),
    /// A file of a source's registry that neither the bundle nor any place behind it serves:
    /// the state is unavailable (U7013), never different, since a bundle's listing does not
    /// commit to the registry files it should hold.
    Unavailable(String),
    /// What a proof needs that neither the bundle nor a place behind it serves, typed by the
    /// client when it opens the copy ([`OpenError::Omitted`]): unavailable (U7013) too.
    Omitted(Omission, String),
}

impl From<Problem> for Failure {
    fn from(problem: Problem) -> Failure {
        match problem {
            Problem::Absent(why) => Failure::Absent(why),
            Problem::Refused(why) => Failure::Refused(why),
        }
    }
}

/// The places behind a bundle's copy of a source: given the source's name, the transports of
/// the other places that serve it, in their order, for the documents the copy omits.
pub type Behind<'b> = dyn Fn(&str) -> Vec<Box<dyn Transport>> + 'b;

/// The copy `copy` as a place, with the places `behind` gives for its source behind it.
fn place_of(copy: &Memory, behind: &Behind<'_>) -> Fallback {
    Fallback::new(Box::new(copy.clone()), behind(&copy.source))
}

/// Opens `copy`, with the places `behind` gives behind it ([`Fallback`]), with the registry
/// client at the resolution point of the packages of its source among `expected`, against
/// `floors`, the checkpoint and the root version Itaoca stores (rules 35 to 39, no freshness),
/// proves that the point extends the lock's ([`Registry::extends`]), and checks that each of
/// those packages is the release the client reads there, by log index and digest, and that its
/// name's yanks and advisories verify. Returns the checkpoint read and the last root version,
/// to store as the source's floors; `Err` gives the package's index and the failure:
/// unavailable when no place serves a document it needs, from the client's typed records
/// ([`OpenError::Unavailable`], [`Registry::take_omitted`]), never from a message.
fn verify_source(
    copy: &Memory,
    behind: &Behind<'_>,
    expected: &[Expected],
    floors: &Floors<'_>,
) -> Result<(String, u64), (usize, Failure)> {
    let place = place_of(copy, behind);
    let mut opened: Option<Registry<'_>> = None;
    for (index, package) in expected.iter().enumerate() {
        if package.source != copy.source {
            continue;
        }
        if opened.is_none() {
            let point = Point::At {
                size: package.checkpoint_size,
                hash: package.checkpoint_hash.clone(),
            };
            match Registry::open_with(&copy.source, &place, &package.root, &point, floors) {
                Ok(mut registry) => {
                    if let Err(why) =
                        registry.extends(package.checkpoint_size, &package.checkpoint_hash)
                    {
                        return Err((index, coded(&mut registry, why)));
                    }
                    opened = Some(registry);
                }
                // Neither the copy nor a place behind it serves the root or the checkpoint.
                Err(OpenError::Unavailable(why)) => {
                    return Err((index, Failure::Unavailable(why)));
                }
                Err(OpenError::Omitted(omission, why)) => {
                    return Err((index, Failure::Omitted(omission, why)));
                }
                Err(error @ OpenError::Refused(_)) => {
                    return Err((index, Failure::Refused(error.message())));
                }
            }
        }
        let Some(registry) = opened.as_mut() else {
            continue;
        };
        if let Err(why) = check_opened(registry, package) {
            return Err((index, coded(registry, why)));
        }
    }
    opened
        .map(|registry| (registry.point.document.clone(), registry.root.version))
        .ok_or_else(|| {
            (
                0,
                Failure::Refused(format!("no package of `{}` is locked", copy.source)),
            )
        })
}

/// A failure of `registry`'s reading: unavailable when the client recorded a document no place
/// serves ([`Registry::take_omitted`]), otherwise a copy that does not verify.
fn coded(registry: &mut Registry<'_>, why: String) -> Failure {
    if registry.take_omitted().is_empty() {
        Failure::Refused(why)
    } else {
        Failure::Unavailable(why)
    }
}

/// The checks of [`verify_source`] on `package`, with the source's registry opened: it is the
/// lock's release, its name's state verifies, and its logged time is read.
fn check_opened(registry: &mut Registry<'_>, package: &Expected) -> Result<(), String> {
    if registry.point.checkpoint.size != package.checkpoint_size {
        return Err("the lock names two resolution points of one source".to_owned());
    }
    let version = crate::version::Version::parse_full(&package.version)?;
    let release = registry.release(&package.name, &version)?;
    if release.index != package.logged || release.statement.digest != package.digest {
        return Err(format!(
            "the registry's `{} {}` is not the lock's: its log index or digest differs",
            package.name, package.version
        ));
    }
    registry.marks(&package.name)?;
    // Its logged time, from the kept checkpoints the bundle carries (rule 33), or a place
    // behind it.
    registry.logged_at(release.index)?;
    Ok(())
}

/// What unbundling did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unbundled {
    /// The packages stored, as `name version`.
    pub stored: Vec<String>,
    /// The entries of the bundle that no locked package uses, or that this module does not
    /// store: documents of the per-name state, and packages the lock does not name.
    pub unused: Vec<String>,
    /// Notes: a floor that could not be stored, or read, which is never a failure.
    pub notes: Vec<String>,
}

/// Unbundles `bytes` into `itaoca` for the registry packages `expected` of a lock: reads the
/// bundle, verifies every package ([`verify_package`]), and only when all verify, stores each
/// archive and provenance. Each source's copy is read as a place with the places `behind`
/// gives behind it ([`Fallback`]), so a registry file the bundle lacks is an omission. `Err`
/// gives the index of the package that failed, or none for the bundle itself, and the failure;
/// then nothing is written.
pub fn unbundle(
    bytes: &[u8],
    itaoca: &Itaoca,
    expected: &[Expected],
    behind: &Behind<'_>,
) -> Result<Unbundled, (Option<usize>, Failure)> {
    let read = read(bytes).map_err(|refusal| (None, Failure::Refused(refusal)))?;
    let mut verified = Vec::with_capacity(expected.len());
    let mut used: BTreeSet<String> = BTreeSet::new();
    for (index, package) in expected.iter().enumerate() {
        let packed = verify_package(bytes, &read, package)
            .map_err(|why| (Some(index), Failure::from(why)))?;
        used.extend(package_path(&package.digest));
        used.extend(provenance_path(
            &package.source,
            &package.digest,
            package.checkpoint_size,
        ));
        verified.push(packed);
    }
    // Each source's copy, opened by the client at the lock's resolution point, against the
    // floor Itaoca stores: the releases are the lock's, and their names' states verify.
    // Each source's floors, kept under its registry's identity (`registry_id`, from the pin).
    let mut floors: Vec<(Memory, Expected, String, u64)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let sources: BTreeSet<&str> = expected.iter().map(|e| e.source.as_str()).collect();
    for source in sources {
        // A copy with no file is a place that omits everything: the places behind it may serve
        // it, and with none, the state is unavailable.
        let copy = memory(bytes, &read, source);
        for path in copy.files.keys() {
            used.insert(format!("registry/{source}/{path}"));
        }
        let Some(package) = expected.iter().find(|e| e.source == source) else {
            continue;
        };
        let id = registry_id(&package.root);
        // A floor that cannot be read is no floor: it is a note, and the copy is still verified.
        let floor = itaoca.load_checkpoint(&id).unwrap_or_else(|why| {
            notes.push(format!("note: the floor of `{source}` does not read: {why}"));
            None
        });
        let root_version = itaoca.load_root_version(&id).unwrap_or_else(|why| {
            notes.push(format!("note: the root floor of `{source}` does not read: {why}"));
            None
        });
        let stored = Floors {
            checkpoint: floor.as_deref(),
            root_version,
        };
        let (document, version) = verify_source(&copy, behind, expected, &stored)
            .map_err(|(index, failure)| (Some(index), failure))?;
        floors.push((copy, package.clone(), document, version));
    }
    let mut stored = Vec::new();
    for (index, (package, packed)) in expected.iter().zip(&verified).enumerate() {
        let fail = |why: String| (Some(index), Failure::Refused(why));
        itaoca
            .store(&packed.archive)
            .map_err(|why| fail(format!("cannot store `{}`: {why}", package.name)))?;
        itaoca
            .store_provenance(
                &package.source,
                &package.name,
                &package.version,
                package.checkpoint_size,
                &packed.provenance,
            )
            .map_err(|why| {
                fail(format!(
                    "cannot store the provenance of `{}`: {why}",
                    package.name
                ))
            })?;
        stored.push(format!("{} {}", package.name, package.version));
    }
    // Through Itaoca's floor API, under its lock, each only forward (rule 39); the stored floor
    // is proven consistent again against the copy, opened anew. A floor that cannot be stored is
    // a note, never a failure: the packages are stored and verified already.
    for (copy, package, document, version) in &floors {
        let source = &copy.source;
        let id = registry_id(&package.root);
        let point = Point::At {
            size: package.checkpoint_size,
            hash: package.checkpoint_hash.clone(),
        };
        let place = place_of(copy, behind);
        match Registry::open_with(source, &place, &package.root, &point, &Floors::default()) {
            Ok(mut registry) => {
                let mut consistent = |stored: &str| registry.consistent_with(stored);
                if let Err(why) = itaoca.store_checkpoint(&id, document, &mut consistent) {
                    notes.push(format!(
                        "note: the checkpoint of `{source}` was not stored as its floor: {why}"
                    ));
                }
            }
            Err(error) => notes.push(format!(
                "note: the checkpoint of `{source}` was not stored as its floor: {}",
                error.message()
            )),
        }
        if let Err(why) = itaoca.store_root_version(&id, *version) {
            notes.push(format!(
                "note: the root version of `{source}` was not stored as its floor: {why}"
            ));
        }
    }
    let unused = read
        .entries
        .keys()
        .filter(|path| !used.contains(*path))
        .cloned()
        .collect();
    Ok(Unbundled {
        stored,
        unused,
        notes,
    })
}
