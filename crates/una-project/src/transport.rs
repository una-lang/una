//! How the registry client reads a registry's files (ADR 0122 rules 34, 39 and 40): a
//! [`Transport`] gives the bytes at a path below a registry's base, and nothing else. Integrity
//! never rests on the transport: every byte it gives is verified by digests, signatures and
//! proofs ([`crate::client`]), whatever the transport is, a local directory included.
//!
//! **Locations.** A registry or a mirror is named by a location ([`Location::parse`]):
//! - an `https://` URL: the network location of a source. This release has no HTTPS transport,
//!   so reading from one fails with a refusal that says so, never a silent skip;
//! - a `file://` URL with an absolute path, written as it is, with no percent-encoding (`%` is
//!   refused): a mirror on the local file system, read by [`Directory`];
//! - any other scheme, `http://` included, and a bare path are refused when they are parsed:
//!   plain HTTP is never used, and neither is any scheme this client does not know.
//!
//! No transport starts another program.
//!
//! Costs: a read is O(b) for the b bytes of the file, bounded by the limit its caller gives.

use std::path::{Path, PathBuf};

use crate::tree;

/// Why a read fails: one sentence.
pub type Refusal = String;

/// Reads the files of one registry, by their paths below its base (rule 34).
pub trait Transport {
    /// The bytes of the file at `path`, of at most `limit` bytes, or `None` when the registry
    /// has no file there. `path` is relative, of `/`-separated segments, none empty, `.` or
    /// `..`. A file larger than `limit` is a refusal, never a truncation.
    fn get(&self, path: &str, limit: u64) -> Result<Option<Vec<u8>>, Refusal>;

    /// Every copy of the file at `path` that the places behind this transport serve, in their
    /// order: none, or the one [`Transport::get`] gives, for a single place. The client merges
    /// the witnesses' cosignatures of every copy of a kept checkpoint.
    fn get_all(&self, path: &str, limit: u64) -> Result<Vec<Vec<u8>>, Refusal> {
        Ok(self.get(path, limit)?.into_iter().collect())
    }

    /// Where the transport reads from, for messages.
    fn describe(&self) -> String;

    /// Every place the transport reads from, in order, for evidence: the one it describes,
    /// for a single place.
    fn describe_all(&self) -> Vec<String> {
        vec![self.describe()]
    }

    /// Whether the transport can serve at all: false for a place this release has no transport
    /// for ([`Unavailable`]), whose every read fails. Such a failure is a refusal of service,
    /// an omission of what was asked (U7013), never evidence that something served does not
    /// verify (U7010).
    fn serves(&self) -> bool {
        true
    }
}

/// Where a registry or a mirror is (rule 40).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// An `https://` base URL, without a trailing `/`.
    Https(String),
    /// A directory on the local file system.
    Directory(PathBuf),
}

impl Location {
    /// Reads a location: an `https://` URL, or a `file://` URL whose path is absolute and holds
    /// no `%`, `?` or `#`. Any other scheme (`http:`, `ftp:`, ...), a bare path, and a URL with
    /// a host, a query, a fragment or credentials where a file URL has none, are refused, so
    /// plain HTTP is never used.
    pub fn parse(text: &str) -> Result<Location, Refusal> {
        if text.is_empty() || text.chars().any(char::is_control) {
            return Err("a registry location is not empty and has no control character".to_owned());
        }
        if let Some(rest) = text.strip_prefix("https://") {
            let host_ok = !rest.is_empty()
                && !rest.starts_with('/')
                && rest.chars().all(|c| c.is_ascii_graphic())
                && !rest.contains(['?', '#', '@']);
            if !host_ok {
                return Err(format!("`{text}` is not an https URL of a registry"));
            }
            return Ok(Location::Https(text.trim_end_matches('/').to_owned()));
        }
        if let Some(rest) = text.strip_prefix("file://") {
            // `file:///abs/path`: an empty host, then an absolute path, written as it is: no
            // percent-encoding, so `%` is refused rather than read two ways.
            if !rest.starts_with('/') || rest.contains(['?', '#', '%']) {
                return Err(format!(
                    "`{text}` is not a file URL of a local directory: write `file:///absolute/path`"
                ));
            }
            return Ok(Location::Directory(PathBuf::from(rest)));
        }
        if let Some((scheme, _)) = text.split_once("://") {
            return Err(format!(
                "`{scheme}://` is not a scheme a registry is read with; a location is an `https://` URL or a `file://` URL of a directory, and plain `http://` is never used"
            ));
        }
        Err(format!(
            "`{text}` is not a location: write an `https://` URL or a `file://` URL of a directory"
        ))
    }
}

/// A transport over a directory on the local file system: a mirror, or a copy of a registry's
/// store directory (rule 40), the latest checkpoint at `checkpoint/latest`. Each file is found with no symbolic link followed below the directory, and
/// read once through a descriptor opened with no link followed. What it gives is verified as
/// what any transport gives: a local directory is trusted for nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directory {
    /// The directory.
    pub root: PathBuf,
}

impl Directory {
    /// The transport over `root`.
    pub fn new(root: &Path) -> Directory {
        Directory {
            root: root.to_path_buf(),
        }
    }
}

/// The segments of a registry path, each checked: not empty, not `.` or `..`, of ASCII
/// letters, digits and `.`, `_`, `-`; at most 16 segments.
pub fn segments(path: &str) -> Result<Vec<&str>, Refusal> {
    let parts: Vec<&str> = path.split('/').collect();
    let valid = !parts.is_empty()
        && parts.len() <= 16
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && part.len() <= 255
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
        });
    if valid {
        Ok(parts)
    } else {
        Err(format!("`{path}` is not a path of a registry"))
    }
}

impl Transport for Directory {
    fn get(&self, path: &str, limit: u64) -> Result<Option<Vec<u8>>, Refusal> {
        let mut parts = segments(path)?;
        // A registry serves both `checkpoint` and `checkpoint/<size>`, which no file system can
        // hold as a file and a directory of one name: on disk the latest is
        // `checkpoint/latest`, a name no size can have, as a registry's own store keeps it.
        if parts == ["checkpoint"] {
            parts = vec!["checkpoint", "latest"];
        }
        tree::not_a_link(&self.root)?;
        match tree::locate(&self.root, &parts)? {
            Some(found) => tree::read_file(&found, limit).map(Some),
            None => Ok(None),
        }
    }

    fn describe(&self) -> String {
        format!("the mirror `{}`", self.root.display())
    }
}

/// The transport of an `https://` location. This release has no HTTPS transport: every read
/// is refused with a message that says so and names the way to read the registry's files from
/// a mirror directory. It is never skipped silently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unavailable {
    /// The URL.
    pub url: String,
}

impl Transport for Unavailable {
    fn get(&self, path: &str, _limit: u64) -> Result<Option<Vec<u8>>, Refusal> {
        segments(path)?;
        Err(format!(
            "`{}` is reached over https, and this release has no https transport; list a mirror directory for its source in Itaoca's `mirrors.una`, as a `file://` URL",
            self.url
        ))
    }

    fn describe(&self) -> String {
        format!("`{}`", self.url)
    }

    fn serves(&self) -> bool {
        false
    }
}

/// A place with the other places of its source behind it (`spec/30 §14` rule 1): the root
/// metadata and the latest `checkpoint` are read from the place alone, since they choose it;
/// every other document, verified against that checkpoint by its hash, signature or proof
/// wherever it comes from, is read from the place, or, when the place omits it or cannot
/// serve it, from the next place that serves it. So an omission by one place never changes a
/// result, and makes it unavailable only when no place serves the document. A place behind the
/// first that cannot serve at all ([`Transport::serves`]) serves nothing, so a document no
/// other place serves is `None`, an omission, never the error of that place.
pub struct Fallback {
    /// The place, then the others, in their order.
    places: Vec<Box<dyn Transport>>,
}

impl Fallback {
    /// The place `first`, then `others`, in their order.
    pub fn new(first: Box<dyn Transport>, others: Vec<Box<dyn Transport>>) -> Fallback {
        let mut places = vec![first];
        places.extend(others);
        Fallback { places }
    }
}

impl Transport for Fallback {
    fn get(&self, path: &str, limit: u64) -> Result<Option<Vec<u8>>, Refusal> {
        let chooses = path == "checkpoint" || path.starts_with("root/");
        let mut failure: Option<Refusal> = None;
        for (at, place) in self.places.iter().enumerate() {
            if chooses && at > 0 {
                break;
            }
            if at > 0 && !place.serves() {
                continue;
            }
            match place.get(path, limit) {
                Ok(Some(bytes)) => return Ok(Some(bytes)),
                Ok(None) => {}
                Err(why) => {
                    failure.get_or_insert(why);
                }
            }
        }
        match failure {
            Some(why) => Err(why),
            None => Ok(None),
        }
    }

    fn get_all(&self, path: &str, limit: u64) -> Result<Vec<Vec<u8>>, Refusal> {
        let mut copies = Vec::new();
        let mut failure: Option<Refusal> = None;
        for (at, place) in self.places.iter().enumerate() {
            if at > 0 && !place.serves() {
                continue;
            }
            match place.get_all(path, limit) {
                Ok(served) => copies.extend(served),
                Err(why) => {
                    failure.get_or_insert(why);
                }
            }
        }
        match failure {
            Some(why) if copies.is_empty() => Err(why),
            _ => Ok(copies),
        }
    }

    fn describe(&self) -> String {
        self.places
            .first()
            .map_or_else(String::new, |place| place.describe())
    }

    fn describe_all(&self) -> Vec<String> {
        self.places
            .iter()
            .flat_map(|place| place.describe_all())
            .collect()
    }

    fn serves(&self) -> bool {
        self.places.iter().any(|place| place.serves())
    }
}

/// The transport of a location.
pub fn open(location: &Location) -> Box<dyn Transport> {
    match location {
        Location::Https(url) => Box::new(Unavailable { url: url.clone() }),
        Location::Directory(dir) => Box::new(Directory::new(dir)),
    }
}

#[cfg(test)]
mod tests {
    use super::{Fallback, Refusal, Transport, Unavailable};

    /// A place that serves `copies` of every file.
    struct Copies(Vec<Vec<u8>>);

    impl Transport for Copies {
        fn get(&self, _path: &str, _limit: u64) -> Result<Option<Vec<u8>>, Refusal> {
            Ok(self.0.first().cloned())
        }

        fn get_all(&self, _path: &str, _limit: u64) -> Result<Vec<Vec<u8>>, Refusal> {
            Ok(self.0.clone())
        }

        fn describe(&self) -> String {
            "copies".to_owned()
        }
    }

    #[test]
    fn fallback_passes_every_copy_of_every_place_through() {
        let first = Copies(vec![b"one".to_vec(), b"two".to_vec()]);
        let second = Copies(vec![b"three".to_vec()]);
        let empty = Copies(Vec::new());
        let fallback = Fallback::new(Box::new(first), vec![Box::new(empty), Box::new(second)]);
        assert_eq!(
            fallback.get_all("checkpoint/3", 64),
            Ok(vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()])
        );
        assert_eq!(fallback.get("checkpoint/3", 64), Ok(Some(b"one".to_vec())));
    }

    #[test]
    fn a_place_behind_that_cannot_serve_serves_nothing() {
        let empty = Copies(Vec::new());
        let https = Unavailable {
            url: "https://registry.example".to_owned(),
        };
        let fallback = Fallback::new(Box::new(empty), vec![Box::new(https)]);
        assert_eq!(fallback.get("tile/0/000", 64), Ok(None));
        assert_eq!(fallback.get_all("checkpoint/3", 64), Ok(Vec::new()));
        assert!(fallback.serves());
    }
}
