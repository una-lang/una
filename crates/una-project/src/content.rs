//! The content of a package (ADR 0122 rules 27 to 29): which files it holds, its canonical
//! listing and content digest, and the archive it travels in.
//!
//! Costs: the listing sorts the f files, O(f log f) comparisons of paths; the digest and the
//! archive are one pass over the bytes, O(B) SHA-256 for B bytes; reading an archive is one
//! pass over it, with memory O(listing + archive), since the archive is read once into memory
//! and every file is a range of it.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::ops::Range;

use sha2::{Digest, Sha256};

/// The most files a package holds (ADR 0122 rule 27).
pub const MAX_FILES: usize = 10_000;
/// The most bytes a package holds, its files together (ADR 0122 rule 27).
pub const MAX_BYTES: u64 = 256 << 20;
/// The most segments of a path.
pub const MAX_SEGMENTS: usize = 32;
/// The most bytes of a path.
pub const MAX_PATH_BYTES: usize = 255;
/// The first line of an archive.
pub const ARCHIVE_MAGIC: &str = "ita-archive/1";

/// The reserved device names of Windows (ADR 0122 rule 27), compared ignoring case and any
/// extension.
const DEVICES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Why a package's content is refused: one sentence (U7010 when reading; a refused publish).
pub type Refusal = String;

/// Whether `path` is a path a package may hold, by its form alone: relative, of segments
/// `[A-Za-z0-9_.-]+`, none `.` or `..`, none starting with `.` or ending with `.`, none a
/// device name of Windows, at most 32 segments and 255 bytes.
pub fn check_path(path: &str) -> Result<(), Refusal> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err(format!(
            "the path `{path}` is empty or longer than {MAX_PATH_BYTES} bytes"
        ));
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() > MAX_SEGMENTS {
        return Err(format!(
            "the path `{path}` has more than {MAX_SEGMENTS} segments"
        ));
    }
    for segment in segments {
        let valid = !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
        if !valid {
            return Err(format!(
                "the path `{path}` has a segment `{segment}` that is not `[A-Za-z0-9_.-]+`"
            ));
        }
        if segment.starts_with('.') {
            return Err(format!(
                "the path `{path}` has a hidden segment, or `.` or `..`"
            ));
        }
        if segment.ends_with('.') {
            return Err(format!("the path `{path}` has a segment ending with `.`"));
        }
        let stem = segment
            .split('.')
            .next()
            .unwrap_or(segment)
            .to_ascii_lowercase();
        if DEVICES.contains(&stem.as_str()) {
            return Err(format!(
                "the path `{path}` has a segment `{segment}` that is a device name of Windows"
            ));
        }
    }
    Ok(())
}

/// Whether a package holds the file at `path` (ADR 0122 rule 27): `ita.una`; every
/// `src/**/*.una`; `README.md`, `CHANGELOG.md`, and files named `LICENSE` or starting with
/// `LICENSE-` or `LICENSE.`. Nothing else: no source of another language, no library, no lock.
pub fn is_package_file(path: &str) -> bool {
    match path {
        "ita.una" | "README.md" | "CHANGELOG.md" | "LICENSE" => true,
        _ if path.starts_with("LICENSE-") || path.starts_with("LICENSE.") => !path.contains('/'),
        _ => path.starts_with("src/") && path.ends_with(".una") && path.len() > "src/.una".len(),
    }
}

/// A file of a package: its path and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// The path, relative to the package, with `/` separators.
    pub path: String,
    /// The contents.
    pub bytes: Vec<u8>,
}

/// The listing of a package (ADR 0122 rule 28): for each file in ascending byte order of its
/// path, the path, a NUL byte, the file's length in decimal, a NUL byte, the lowercase hex
/// SHA-256 of its contents, and a line feed. Refused for a file a package may not hold, two
/// paths equal ignoring ASCII case, a path that is, ignoring case, a directory prefix of
/// another, and the bounds of rule 27. The files are sorted in place.
pub fn listing(files: &mut [File]) -> Result<String, Refusal> {
    if files.len() > MAX_FILES {
        return Err(format!("the package holds more than {MAX_FILES} files"));
    }
    files.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    check_paths(files.iter().map(|f| f.path.as_str()))?;
    let total: u64 = files.iter().map(|f| f.bytes.len() as u64).sum();
    if total > MAX_BYTES {
        return Err(format!("the package holds more than {MAX_BYTES} bytes"));
    }
    let mut out = String::new();
    for file in files.iter() {
        let hash = hex(&Sha256::digest(&file.bytes));
        let _ = writeln_entry(&mut out, &file.path, file.bytes.len() as u64, &hash);
    }
    Ok(out)
}

fn writeln_entry(out: &mut String, path: &str, len: u64, hash: &str) -> std::fmt::Result {
    writeln!(out, "{path}\0{len}\0{hash}")
}

/// The paths' rules taken together, over paths in ascending byte order: each one's form, a
/// package file, unique ignoring ASCII case, and none a directory prefix of another ignoring
/// case. O(f log f) with a set of the lowercased paths and their directories.
fn check_paths<'a>(paths: impl Iterator<Item = &'a str>) -> Result<(), Refusal> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut directories: BTreeSet<String> = BTreeSet::new();
    let mut lower_paths: Vec<String> = Vec::new();
    let mut previous: Option<&str> = None;
    for path in paths {
        if previous.is_some_and(|p| p.as_bytes() >= path.as_bytes()) {
            return Err(format!("the path `{path}` is out of order or repeated"));
        }
        previous = Some(path);
        check_path(path)?;
        if !is_package_file(path) {
            return Err(format!(
                "`{path}` is not a file a package holds: ita.una, src/**/*.una, README.md, CHANGELOG.md and LICENSE files only"
            ));
        }
        let lower = path.to_ascii_lowercase();
        if !seen.insert(lower.clone()) {
            return Err(format!("two paths are `{path}` ignoring case"));
        }
        let mut at = 0;
        while let Some(slash) = lower.get(at..).and_then(|rest| rest.find('/')) {
            directories.insert(lower.get(..at + slash).unwrap_or("").to_owned());
            at += slash + 1;
        }
        lower_paths.push(lower);
    }
    for lower in &lower_paths {
        if directories.contains(lower) {
            return Err(format!(
                "the path `{lower}` is also a directory of another path, ignoring case"
            ));
        }
    }
    Ok(())
}

/// The content digest of a listing: `sha256:` and the hex SHA-256 of it (ADR 0122 rule 28).
pub fn digest(listing: &str) -> String {
    format!("sha256:{}", hex(&Sha256::digest(listing.as_bytes())))
}

/// The digest of `files`, through their listing.
pub fn digest_of(files: &mut [File]) -> Result<String, Refusal> {
    listing(files).map(|listing| digest(&listing))
}

/// The archive of a package (ADR 0122 rule 29): the line `ita-archive/1`, the listing's length
/// in decimal and a line feed, the listing, then each file's bytes in listing order, and
/// nothing after. No compression.
pub fn archive(files: &mut [File]) -> Result<Vec<u8>, Refusal> {
    let listing = listing(files)?;
    let mut out = Vec::new();
    out.extend_from_slice(ARCHIVE_MAGIC.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(listing.len().to_string().as_bytes());
    out.push(b'\n');
    out.extend_from_slice(listing.as_bytes());
    for file in files.iter() {
        out.extend_from_slice(&file.bytes);
    }
    Ok(out)
}

/// An archive read: its digest, and each file's path and range of the archive's bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    /// The content digest, computed from the bytes read.
    pub digest: String,
    /// Each file, in listing order, with the range of its bytes in the archive.
    pub files: Vec<(String, Range<usize>)>,
}

/// Reads an archive (ADR 0122 rule 29) from its bytes, read once into memory: the listing is
/// parsed and every file hashed from those same bytes. Refused: another first line; a length
/// with a leading zero, above the bounds, or past the end; a listing that does not end with a
/// line feed, whose paths are not in strictly ascending byte order or that rule 27 refuses; a
/// file whose hash differs from the listing's; trailing bytes. O(size).
pub fn read_archive(bytes: &[u8]) -> Result<Read, Refusal> {
    let mut at = 0;
    let line = next_line(bytes, &mut at).ok_or("the archive has no first line")?;
    if line != ARCHIVE_MAGIC.as_bytes() {
        return Err(format!("the archive does not start with `{ARCHIVE_MAGIC}`"));
    }
    let length = next_line(bytes, &mut at).ok_or("the archive has no listing length")?;
    let length =
        decimal(length).ok_or("the listing's length is not a decimal without a leading zero")?;
    if length > MAX_BYTES {
        return Err("the listing is longer than a package may be".to_owned());
    }
    let length = usize::try_from(length).map_err(|_| "the listing is too long".to_owned())?;
    let end = at
        .checked_add(length)
        .filter(|end| *end <= bytes.len())
        .ok_or("the listing's length runs past the end of the archive")?;
    let listing = std::str::from_utf8(bytes.get(at..end).unwrap_or_default())
        .map_err(|_| "the listing is not UTF-8".to_owned())?;
    if !listing.is_empty() && !listing.ends_with('\n') {
        return Err("the listing does not end with a line feed".to_owned());
    }
    let mut entries: Vec<(&str, u64, &str)> = Vec::new();
    for entry in listing.lines() {
        let mut parts = entry.split('\0');
        let (Some(path), Some(len), Some(hash), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err("an entry of the listing is not `path NUL length NUL hash`".to_owned());
        };
        let len = decimal(len.as_bytes())
            .ok_or("a file's length is not a decimal without a leading zero")?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(format!(
                "the hash of `{path}` is not 64 lowercase hexadecimal digits"
            ));
        }
        entries.push((path, len, hash));
    }
    if entries.len() > MAX_FILES {
        return Err(format!("the archive holds more than {MAX_FILES} files"));
    }
    check_paths(entries.iter().map(|(path, _, _)| *path))?;
    let mut cursor = end;
    let mut total: u64 = 0;
    let mut files = Vec::with_capacity(entries.len());
    for (path, len, hash) in entries {
        total = total.saturating_add(len);
        if total > MAX_BYTES {
            return Err(format!("the archive holds more than {MAX_BYTES} bytes"));
        }
        let len = usize::try_from(len).map_err(|_| "a file is too long".to_owned())?;
        let stop = cursor
            .checked_add(len)
            .filter(|stop| *stop <= bytes.len())
            .ok_or_else(|| format!("the length of `{path}` runs past the end of the archive"))?;
        let contents = bytes.get(cursor..stop).unwrap_or_default();
        if hex(&Sha256::digest(contents)) != hash {
            return Err(format!(
                "the bytes of `{path}` do not have the hash the listing gives"
            ));
        }
        files.push((path.to_owned(), cursor..stop));
        cursor = stop;
    }
    if cursor != bytes.len() {
        return Err("the archive has bytes after its last file".to_owned());
    }
    Ok(Read {
        digest: digest(listing),
        files,
    })
}

/// The bytes up to the next line feed, which is consumed; `None` at the end.
fn next_line<'b>(bytes: &'b [u8], at: &mut usize) -> Option<&'b [u8]> {
    let rest = bytes.get(*at..)?;
    let newline = rest.iter().position(|b| *b == b'\n')?;
    let line = rest.get(..newline)?;
    *at += newline + 1;
    Some(line)
}

/// A decimal without a leading zero, at most 20 digits.
fn decimal(text: &[u8]) -> Option<u64> {
    let valid = !text.is_empty()
        && text.len() <= 20
        && text.iter().all(u8::is_ascii_digit)
        && (text.len() == 1 || text.first() != Some(&b'0'));
    if !valid {
        return None;
    }
    std::str::from_utf8(text).ok()?.parse().ok()
}

/// Lowercase hexadecimal.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
