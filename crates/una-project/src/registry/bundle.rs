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
//!   per-name state, the owners' keys and every inclusion proof travel with it.
//!
//! **Nothing in a bundle is trusted.** Unbundling reads the bundle once into memory, checks its
//! listing and every entry's hash, and then, for every registry package of the lock, verifies
//! its archive's digest against the lock and its provenance as every load does (rule 17); and
//! opens each source's copy ([`Memory`]) with the registry client at the lock's resolution point
//! (rules 35 to 39): the root chain from the pin of `ita.una`, the checkpoint's log signature and
//! witnesses, the consistency with the checkpoint Itaoca stores for the source, the name's whole
//! chain with its yanks and advisories, and the owner's keys. The release the client reads MUST
//! be the lock's: its log index and digest. Only when every package verifies is anything
//! written; then the source's checkpoint is stored as its floor, which never moves backwards. No
//! clock freshness is asked: the lock's digests and the proofs decide.
//!
//! Costs: O(B) for a bundle of B bytes, read and hashed once, with memory O(B); O(e log e) to
//! sort e entries; and per package the provenance's O(log L) hashes.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use sha2::{Digest, Sha256};

use super::{decimal, digest_hex, hex};
use crate::client::{Point, Registry};
use crate::content::check_path;
use crate::itaoca::Itaoca;
use crate::provenance::{self, Expected};
use crate::transport::{Refusal, Transport, segments};
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
    verify_parts(archive, provenance, &what, expected).map_err(refused)
}

fn verify_parts(
    archive: &[u8],
    provenance: &[u8],
    what: &str,
    expected: &Expected,
) -> Result<Packed, String> {
    let tree = VerifiedTree::from_archive(archive)
        .map_err(|refusal| format!("the archive of {what} in the bundle: {refusal}"))?;
    if tree.digest() != expected.digest {
        return Err(format!(
            "the archive of {what} in the bundle has the digest {}, not the lock's {}",
            tree.digest(),
            expected.digest
        ));
    }
    let document = provenance::parse(provenance)
        .map_err(|refusal| format!("the provenance of {what} in the bundle: {refusal}"))?;
    let statement = provenance::verify(&document, expected)
        .map_err(|refusal| format!("the provenance of {what} in the bundle: {refusal}"))?;
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

/// Opens `copy` with the registry client at the resolution point of the packages of its source
/// among `expected`, with `floor` as the stored checkpoint (rules 35 to 39, no freshness), and
/// checks that each of those packages is the release the client reads there, by log index and
/// digest, and that its name's yanks and advisories verify. Returns the checkpoint read, to
/// store as the source's floor; `Err` gives the package's index and why.
fn verify_source(
    copy: &Memory,
    expected: &[Expected],
    floor: Option<&str>,
) -> Result<String, (usize, String)> {
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
            let registry = Registry::open(&copy.source, copy, &package.root, &point, floor)
                .map_err(|why| (index, why))?;
            opened = Some(registry);
        }
        let Some(registry) = opened.as_mut() else {
            continue;
        };
        if registry.point.checkpoint.size != package.checkpoint_size {
            return Err((
                index,
                "the lock names two resolution points of one source".to_owned(),
            ));
        }
        let version =
            crate::version::Version::parse_full(&package.version).map_err(|why| (index, why))?;
        let release = registry
            .release(&package.name, &version)
            .map_err(|why| (index, why))?;
        if release.index != package.logged || release.statement.digest != package.digest {
            return Err((
                index,
                format!(
                    "the registry's `{} {}` is not the lock's: its log index or digest differs",
                    package.name, package.version
                ),
            ));
        }
        registry.marks(&package.name).map_err(|why| (index, why))?;
    }
    opened
        .map(|registry| registry.point.document.clone())
        .ok_or_else(|| (0, format!("no package of `{}` is locked", copy.source)))
}

/// What unbundling did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unbundled {
    /// The packages stored, as `name version`.
    pub stored: Vec<String>,
    /// The entries of the bundle that no locked package uses, or that this module does not
    /// store: documents of the per-name state, and packages the lock does not name.
    pub unused: Vec<String>,
}

/// Unbundles `bytes` into `itaoca` for the registry packages `expected` of a lock: reads the
/// bundle, verifies every package ([`verify_package`]), and only when all verify, stores each
/// archive and provenance. `Err` gives the index of the package that failed, or none for the
/// bundle itself, and why; then nothing is written.
pub fn unbundle(
    bytes: &[u8],
    itaoca: &Itaoca,
    expected: &[Expected],
) -> Result<Unbundled, (Option<usize>, Problem)> {
    let read = read(bytes).map_err(|refusal| (None, Problem::Refused(refusal)))?;
    let mut verified = Vec::with_capacity(expected.len());
    let mut used: BTreeSet<String> = BTreeSet::new();
    for (index, package) in expected.iter().enumerate() {
        let packed = verify_package(bytes, &read, package).map_err(|why| (Some(index), why))?;
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
    let mut floors: Vec<(String, String)> = Vec::new();
    let sources: BTreeSet<&str> = expected.iter().map(|e| e.source.as_str()).collect();
    for source in sources {
        let copy = memory(bytes, &read, source);
        let first = expected.iter().position(|e| e.source == source);
        if copy.files.is_empty() {
            return Err((
                first,
                Problem::Absent(format!(
                    "the bundle holds no copy of the registry `{source}`, whose state its packages need"
                )),
            ));
        }
        for path in copy.files.keys() {
            used.insert(format!("registry/{source}/{path}"));
        }
        let floor = itaoca
            .load_checkpoint(source)
            .map_err(|why| (first, Problem::Refused(why)))?;
        let document = verify_source(&copy, expected, floor.as_deref())
            .map_err(|(index, why)| (Some(index), Problem::Refused(why)))?;
        floors.push((source.to_owned(), document));
    }
    let mut stored = Vec::new();
    for (index, (package, packed)) in expected.iter().zip(&verified).enumerate() {
        let fail = |why: String| (Some(index), Problem::Refused(why));
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
    for (source, document) in &floors {
        itaoca
            .store_checkpoint(source, document)
            .map_err(|why| (None, Problem::Refused(why)))?;
    }
    let unused = read
        .entries
        .keys()
        .filter(|path| !used.contains(*path))
        .cloned()
        .collect();
    Ok(Unbundled { stored, unused })
}
