//! Host capabilities (`spec/14 §3`): the world that performs input and output, the grants of
//! `una run`, the confinement of `Files` to its prefix, and HMAC-SHA-256 for `Keys`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

/// What a file operation of the world can fail with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileFault {
    /// The file does not exist.
    NotFound,
    /// Any other input or output error.
    Io,
    /// The file's content is not valid UTF-8: the run aborts with U6005, since the content,
    /// not the path or the file system's shape, would decide a failure (S-16, ADR 0086).
    NotText,
}

/// A host value that is not valid UTF-8, which the run aborts on with U6005 (S-16, ADR 0086).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotText;

/// The world outside the program: the only place where host operations touch the machine
/// they run on. `una-cli` implements it; tests implement it in memory.
pub trait World {
    /// `Console.print`: writes `line` and a newline.
    fn print(&mut self, line: &str);
    /// `Clock.now`: milliseconds since the Unix epoch.
    fn now(&mut self) -> i64;
    /// `Env.get`: an environment variable.
    fn env_var(&mut self, name: &str) -> Result<Option<String>, NotText>;
    /// Reads a file as text: [`FileFault::NotText`] when its content is not valid UTF-8.
    fn read_file(&mut self, path: &Path) -> Result<String, FileFault>;
    /// Writes a file. It never creates a file through a symbolic link (`spec/14 §3`): a file
    /// that does not exist is created only as a new file at `path` itself.
    fn write_file(&mut self, path: &Path, content: &str) -> Result<(), FileFault>;
    /// The canonical form of an existing path, following links; `None` when it does not
    /// exist.
    fn canonical(&mut self, path: &Path) -> Option<PathBuf>;
    /// Whether `path` itself is a symbolic link, dangling or not, without following it.
    fn is_link(&mut self, path: &Path) -> bool;
    /// Whether `path`, which [`World::canonical`] did not resolve, does not exist: a component
    /// of it is missing or is not a directory. Any other failure to resolve it, a host limit
    /// (a resolved path longer than the host takes, more links than it follows) or a
    /// directory that may not be searched, is not absence, and [`confine`] denies the path
    /// (`spec/14 §3` rule 2, #1380): the operation would otherwise open the lexical path,
    /// through links that confinement never checked. The default is `false`, so a world that
    /// does not tell denies every path whose directory does not resolve.
    fn absent(&mut self, _path: &Path) -> bool {
        false
    }
}

/// The world of examples: they cannot reach host capabilities (`spec/11 §6`), so nothing here
/// is ever called.
pub(crate) struct NoWorld;

impl World for NoWorld {
    fn print(&mut self, _line: &str) {}

    fn now(&mut self) -> i64 {
        0
    }

    fn env_var(&mut self, _name: &str) -> Result<Option<String>, NotText> {
        Ok(None)
    }

    fn read_file(&mut self, _path: &Path) -> Result<String, FileFault> {
        Err(FileFault::NotFound)
    }

    fn write_file(&mut self, _path: &Path, _content: &str) -> Result<(), FileFault> {
        Err(FileFault::Io)
    }

    fn canonical(&mut self, _path: &Path) -> Option<PathBuf> {
        None
    }

    fn is_link(&mut self, _path: &Path) -> bool {
        false
    }
}

/// A grant of `una run` (`spec/13 §5`), with its paths already resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grant {
    /// `console`.
    Console,
    /// `clock`.
    Clock,
    /// `env`.
    Env,
    /// `files:<path>:<modes>`.
    Files {
        /// The prefix that paths are relative to.
        prefix: PathBuf,
        /// Whether reading is allowed.
        read: bool,
        /// Whether writing is allowed.
        write: bool,
    },
    /// `keys:<file>`.
    Keys {
        /// The key file.
        file: PathBuf,
    },
}

impl Grant {
    /// The host capability this grant gives.
    pub fn capability(&self) -> &'static str {
        match self {
            Grant::Console => "Console",
            Grant::Clock => "Clock",
            Grant::Env => "Env",
            Grant::Files { .. } => "Files",
            Grant::Keys { .. } => "Keys",
        }
    }

    /// Parses a grant spec of `spec/13 §5`, resolving relative paths against `base`.
    pub fn parse(spec: &str, base: &Path) -> Option<Grant> {
        match spec {
            "console" => Some(Grant::Console),
            "clock" => Some(Grant::Clock),
            "env" => Some(Grant::Env),
            _ => {
                if let Some(file) = spec.strip_prefix("keys:") {
                    return (!file.is_empty()).then(|| Grant::Keys {
                        file: base.join(file),
                    });
                }
                let rest = spec.strip_prefix("files:")?;
                let (path, modes) = rest.rsplit_once(':')?;
                let (read, write) = match modes {
                    "read" => (true, false),
                    "write" => (false, true),
                    "read,write" => (true, true),
                    _ => return None,
                };
                (!path.is_empty()).then(|| Grant::Files {
                    prefix: normalize(&base.join(path)),
                    read,
                    write,
                })
            }
        }
    }
}

/// `path` with `.` removed and `..` applied, lexically.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The path that a `Files` operation on `relative` opens under `prefix`, or `None` when the
/// operation is `Denied` (`spec/14 §3`, ADR 0038 D9):
///
/// 1. `relative` is normalized lexically, without following links; an absolute path, a `..`
///    that leaves the prefix, and a path with no final component are denied;
/// 2. every intermediate directory, the prefix included, is resolved with its links followed
///    and must lie inside the canonical prefix; a dangling link there is denied, and a
///    component that does not exist ([`World::absent`]) leaves the rest to the operation,
///    which fails with `NotFound` (or `Io` for a component that is not a directory); a
///    directory that does not resolve for any other reason, a host limit included, is
///    denied (#1380);
/// 3. the final component is never followed: a symbolic link there, dangling or not, is
///    denied. The world opens the returned path without creating a file through a link.
///
/// Races between this check and the operation are outside the v0.1 model (D9 rule 4).
pub fn confine(world: &mut dyn World, prefix: &Path, relative: &str) -> Option<PathBuf> {
    // Rule 1: lexical normalization.
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    // DECISION D-133: a path that names the prefix itself (``, `.`, `a/..`) has no final
    // component to open, and is denied.
    let (last, directories) = parts.split_last()?;
    // Rule 2: every intermediate directory resolves inside the canonical prefix.
    // DECISION D-134: each one is checked, not only the last, so a path never passes through
    // a directory outside the prefix even when it comes back inside.
    let mut lexical = prefix.to_path_buf();
    let Some(root) = world.canonical(prefix) else {
        return missing(world, &lexical, prefix, &parts);
    };
    let mut resolved = root.clone();
    for part in directories {
        lexical.push(part);
        match world.canonical(&lexical) {
            Some(directory) if directory.starts_with(&root) => resolved = directory,
            Some(_) => return None,
            None => return missing(world, &lexical, prefix, &parts),
        }
    }
    // Rule 3: the final component is never followed.
    let target = resolved.join(last);
    (!world.is_link(&target)).then_some(target)
}

/// [`World::absent`] on the machine's file system, for the worlds that reach the disk: the
/// canonicalization of `path` fails because a component is missing (`ENOENT`) or is not a
/// directory (`ENOTDIR`). Any other failure (`ENAMETOOLONG`, `ELOOP`, `EACCES`, ...), or a
/// canonicalization that now succeeds, is not absence.
pub fn disk_absent(path: &Path) -> bool {
    std::fs::canonicalize(path).is_err_and(|error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        )
    })
}

/// The outcome of [`confine`] when the intermediate directory `lexical` does not resolve: a
/// dangling link is denied, a directory that does not exist yields the lexical path, whose
/// operation then fails with `NotFound` without touching anything outside the prefix, and
/// any other failure to resolve is denied (#1380): the lexical path would be opened through
/// links that were never checked.
fn missing(
    world: &mut dyn World,
    lexical: &Path,
    prefix: &Path,
    parts: &[&std::ffi::OsStr],
) -> Option<PathBuf> {
    if world.is_link(lexical) || !world.absent(lexical) {
        return None;
    }
    let mut out = prefix.to_path_buf();
    out.extend(parts);
    Some(out)
}

/// The keys of a key file: a TOML table of key names to base64 keys. The
/// error names what is wrong, in one sentence.
pub fn parse_keys(text: &str) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let table: BTreeMap<String, toml::Value> = toml::from_str(text)
        .map_err(|error| format!("it is not valid TOML: {}", error.message()))?;
    let mut keys = BTreeMap::new();
    for (name, value) in table {
        let toml::Value::String(encoded) = value else {
            return Err(format!("the key `{name}` is not a base64 string"));
        };
        match base64(&encoded) {
            Some(key) if !key.is_empty() => {
                keys.insert(name, key);
            }
            _ => return Err(format!("the key `{name}` is not a nonempty base64 string")),
        }
    }
    Ok(keys)
}

/// HMAC-SHA-256 (RFC 2104) over `sha2`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut block = if key.len() > BLOCK {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    block.resize(BLOCK, 0);
    let inner_pad: Vec<u8> = block.iter().map(|b| b ^ 0x36).collect();
    let outer_pad: Vec<u8> = block.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::new()
        .chain_update(&inner_pad)
        .chain_update(data)
        .finalize();
    Sha256::new()
        .chain_update(&outer_pad)
        .chain_update(inner)
        .finalize()
        .to_vec()
}

/// The deterministic `Hashed` value of `std.testing.fake_hash(seed)` (`spec/14 §4`).
pub fn fake_hash(seed: &str) -> Vec<u8> {
    Sha256::digest(format!("una.testing.fake_hash:{seed}").as_bytes()).to_vec()
}

/// Lowercase hexadecimal.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes standard base64, with or without padding.
pub fn base64(text: &str) -> Option<Vec<u8>> {
    let mut bits: u32 = 0;
    let mut count = 0;
    let mut out = Vec::new();
    for c in text.trim().chars() {
        let value = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            '=' => break,
            _ => return None,
        };
        bits = (bits << 6) | value;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push(((bits >> count) & 0xff) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 2 of RFC 4231.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: a key longer than the block.
        let key = [0xaa_u8; 131];
        let mac = hmac_sha256(
            &key,
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        );
        assert_eq!(
            hex(&mac),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn base64_decodes_with_and_without_padding() {
        assert_eq!(base64("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(base64("aGVsbG8"), Some(b"hello".to_vec()));
        assert_eq!(base64("*"), None);
    }

    #[test]
    fn key_files_are_tables_of_base64_keys() {
        let keys = parse_keys("phone_pepper = \"aGVsbG8=\"\n").unwrap();
        assert_eq!(keys.get("phone_pepper"), Some(&b"hello".to_vec()));
        assert!(parse_keys("phone_pepper = 3\n").is_err());
        assert!(parse_keys("phone_pepper = \"*\"\n").is_err());
        assert!(parse_keys("= broken").is_err());
    }

    #[test]
    fn grants_parse_and_resolve_their_paths() {
        let base = Path::new("/work/app");
        assert_eq!(Grant::parse("console", base), Some(Grant::Console));
        assert_eq!(
            Grant::parse("files:./data:read,write", base),
            Some(Grant::Files {
                prefix: PathBuf::from("/work/app/data"),
                read: true,
                write: true
            })
        );
        assert_eq!(
            Grant::parse("keys:keys.toml", base),
            Some(Grant::Keys {
                file: PathBuf::from("/work/app/keys.toml")
            })
        );
        assert_eq!(Grant::parse("files:./data:execute", base), None);
        assert_eq!(Grant::parse("network", base), None);
    }

    /// A node of the in-memory file system of the #1380 tests.
    enum Node {
        Dir,
        File,
        Link(String),
    }

    /// Why a resolution of the model stops.
    #[derive(Debug, PartialEq, Eq)]
    enum Fault {
        /// A name is missing (`ENOENT`).
        Missing,
        /// A name is looked up in a file (`ENOTDIR`).
        NotDir,
        /// More than 40 links (`ELOOP`).
        Loop,
        /// A resolved path longer than `max_path` (`ENAMETOOLONG`).
        TooLong,
    }

    /// An in-memory file system with the semantics of `realpath` and `lstat`. Its
    /// canonicalization fails, as `realpath` does, when a path it resolves grows longer than
    /// `max_path` bytes, though the lexical path stays short. With `before`, every path that
    /// does not resolve counts as absent: the rule before #1380.
    struct Model {
        nodes: BTreeMap<PathBuf, Node>,
        max_path: usize,
        before: bool,
    }

    impl Model {
        fn new() -> Model {
            let mut nodes = BTreeMap::new();
            nodes.insert(PathBuf::from("/"), Node::Dir);
            Model {
                nodes,
                max_path: usize::MAX,
                before: false,
            }
        }

        /// Adds `node` at the absolute `path`, with the directories above it.
        fn add(&mut self, path: &str, node: Node) {
            let path = PathBuf::from(path);
            for ancestor in path.ancestors().skip(1) {
                self.nodes
                    .entry(ancestor.to_path_buf())
                    .or_insert(Node::Dir);
            }
            self.nodes.insert(path, node);
        }

        /// Walks `path` from the root, following links (the last one only when
        /// `follow_last`), with at most 40 links followed and resolved paths of at most
        /// `limit` bytes.
        fn resolve(&self, path: &Path, follow_last: bool, limit: usize) -> Result<PathBuf, Fault> {
            let mut pending: VecDeque<String> = path
                .to_str()
                .unwrap()
                .split('/')
                .map(str::to_owned)
                .collect();
            let mut at = PathBuf::from("/");
            let mut hops = 0;
            while let Some(name) = pending.pop_front() {
                match name.as_str() {
                    "" | "." => continue,
                    ".." => {
                        at.pop();
                        continue;
                    }
                    _ => {}
                }
                if !matches!(self.nodes.get(&at), Some(Node::Dir)) {
                    return Err(Fault::NotDir);
                }
                let next = at.join(&name);
                match self.nodes.get(&next) {
                    None => return Err(Fault::Missing),
                    Some(Node::Link(target)) if follow_last || !pending.is_empty() => {
                        hops += 1;
                        if hops > 40 {
                            return Err(Fault::Loop);
                        }
                        if target.starts_with('/') {
                            at = PathBuf::from("/");
                        }
                        for part in target.split('/').rev() {
                            pending.push_front(part.to_owned());
                        }
                    }
                    Some(_) => {
                        at = next;
                        if at.as_os_str().len() > limit {
                            return Err(Fault::TooLong);
                        }
                    }
                }
            }
            Ok(at)
        }
    }

    /// The `World` methods confinement never calls.
    macro_rules! no_io {
        () => {
            fn print(&mut self, _line: &str) {}
            fn now(&mut self) -> i64 {
                0
            }
            fn env_var(&mut self, _name: &str) -> Result<Option<String>, NotText> {
                Ok(None)
            }
            fn read_file(&mut self, _path: &Path) -> Result<String, FileFault> {
                Err(FileFault::Io)
            }
            fn write_file(&mut self, _path: &Path, _content: &str) -> Result<(), FileFault> {
                Err(FileFault::Io)
            }
        };
    }

    impl World for Model {
        no_io!();
        fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
            self.resolve(path, true, self.max_path).ok()
        }
        fn is_link(&mut self, path: &Path) -> bool {
            self.resolve(path, false, usize::MAX)
                .is_ok_and(|at| matches!(self.nodes.get(&at), Some(Node::Link(_))))
        }
        fn absent(&mut self, path: &Path) -> bool {
            self.before
                || matches!(
                    self.resolve(path, true, self.max_path),
                    Err(Fault::Missing | Fault::NotDir)
                )
        }
    }

    /// The model, with the default [`World::absent`].
    struct Silent<'a>(&'a mut Model);

    impl World for Silent<'_> {
        no_io!();
        fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
            self.0.canonical(path)
        }
        fn is_link(&mut self, path: &Path) -> bool {
            self.0.is_link(path)
        }
    }

    #[test]
    fn a_directory_past_the_host_path_limit_is_denied_1380() {
        // `spec/14 §3` rule 2, #1380: a link inside the prefix resolves to a deep directory,
        // and the canonical path of the directory below it is longer than the host takes
        // (1024 bytes, as on macOS), so it does not resolve though it exists. Below it a link
        // leads outside the prefix. The rule before #1380 took the unresolved directory for a
        // missing one and returned the lexical path, which the operation opened through that
        // link.
        let name = "X".repeat(200);
        let deep = |levels: usize| vec![name.as_str(); levels].join("/");
        let mut model = Model::new();
        model.max_path = 1024;
        model.add("/r/outside/secret.txt", Node::File);
        model.add("/r/box/note.txt", Node::File);
        model.add(
            &format!("/r/box/{}/esc", deep(6)),
            Node::Link("/r/outside".to_owned()),
        );
        model.add("/r/box/q", Node::Link(deep(4)));
        let prefix = Path::new("/r/box");
        let relative = format!("q/{}/esc/secret.txt", deep(2));
        // `q/X` resolves (1011 bytes), `q/X/X` does not (1212 bytes) and is not a link.
        assert!(model.canonical(&prefix.join(format!("q/{name}"))).is_some());
        let past = prefix.join(format!("q/{}", deep(2)));
        assert_eq!(
            model.resolve(&past, true, model.max_path).err(),
            Some(Fault::TooLong)
        );
        assert!(!model.is_link(&past));
        // The rule before #1380 returned the lexical path, which leads outside the prefix.
        model.before = true;
        let before = confine(&mut model, prefix, &relative);
        assert_eq!(
            before,
            Some(prefix.join(&relative)),
            "the rule before #1380"
        );
        model.before = false;
        model.max_path = usize::MAX;
        assert_eq!(
            model.canonical(&before.unwrap()),
            Some(PathBuf::from("/r/outside/secret.txt")),
            "the lexical path leads outside the prefix"
        );
        model.max_path = 1024;
        // Denied, and denied even with nothing below the directory past the limit.
        assert_eq!(confine(&mut model, prefix, &relative), None);
        let new = format!("q/{}/new.txt", deep(2));
        assert_eq!(confine(&mut model, prefix, &new), None);
        // Directories that do not exist, or are not directories, still leave the path to the
        // operation, under the deep link too.
        model.add(&format!("/r/box/{}/file.txt", deep(5)), Node::File);
        for relative in [
            "missing/new.txt".to_owned(),
            "missing/deeper/new.txt".to_owned(),
            "note.txt/x/new.txt".to_owned(),
            "q/missing/new.txt".to_owned(),
            format!("q/{name}/missing/new.txt"),
            format!("q/{name}/file.txt/x/new.txt"),
        ] {
            assert_eq!(
                confine(&mut model, prefix, &relative),
                Some(prefix.join(&relative)),
                "{relative}"
            );
        }
        // Paths that resolve keep their outcomes.
        assert_eq!(
            confine(&mut model, prefix, "note.txt"),
            Some(PathBuf::from("/r/box/note.txt"))
        );
        assert_eq!(
            confine(&mut model, prefix, &format!("q/{name}/file.txt")),
            Some(PathBuf::from(format!("/r/box/{}/file.txt", deep(5))))
        );
        // A world that does not tell absence denies every directory that does not resolve.
        assert_eq!(
            confine(&mut Silent(&mut model), prefix, "missing/new.txt"),
            None
        );
        assert_eq!(
            confine(&mut Silent(&mut model), prefix, "note.txt"),
            Some(PathBuf::from("/r/box/note.txt"))
        );
    }

    #[test]
    fn disk_absence_is_a_missing_component_or_one_that_is_not_a_directory_1380() {
        // `disk_absent` holds for `ENOENT` and `ENOTDIR` only, and never for a path that
        // resolves.
        let root = std::env::temp_dir().join(format!("una-absent-1380-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("dir")).unwrap();
        std::fs::write(root.join("file.txt"), "").unwrap();
        assert!(disk_absent(&root.join("missing")));
        assert!(disk_absent(&root.join("missing/deeper")));
        assert!(disk_absent(&root.join("file.txt/x")));
        assert!(!disk_absent(&root.join("dir")));
        assert!(!disk_absent(&root.join("file.txt")));
        // A name longer than the host takes exists in no directory, but is not absence.
        assert!(!disk_absent(&root.join("X".repeat(300)).join("x")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
