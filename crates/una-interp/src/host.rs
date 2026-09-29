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
}

/// The world outside the program: the only place where host operations touch the machine
/// they run on. `una-cli` implements it; tests implement it in memory.
pub trait World {
    /// `Console.print`: writes `line` and a newline.
    fn print(&mut self, line: &str);
    /// `Clock.now`: milliseconds since the Unix epoch.
    fn now(&mut self) -> i64;
    /// `Env.get`: an environment variable.
    fn env_var(&mut self, name: &str) -> Option<String>;
    /// Reads a file as text.
    fn read_file(&mut self, path: &Path) -> Result<String, FileFault>;
    /// Writes a file.
    fn write_file(&mut self, path: &Path, content: &str) -> Result<(), FileFault>;
    /// The canonical form of an existing path, following links; `None` when it does not
    /// exist.
    fn canonical(&mut self, path: &Path) -> Option<PathBuf>;
}

/// The world of examples: they cannot reach host capabilities (`spec/11 §6`), so nothing here
/// is ever called.
pub(crate) struct NoWorld;

impl World for NoWorld {
    fn print(&mut self, _line: &str) {}

    fn now(&mut self) -> i64 {
        0
    }

    fn env_var(&mut self, _name: &str) -> Option<String> {
        None
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
}

/// A grant of `una run` (`spec/13 §5`), with its paths already resolved (DECISION D-103).
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

/// The path of `relative` under `prefix`, or `None` when it leads outside the prefix:
/// absolute, through `..`, or through a link (`spec/14 §3`, DECISION D-103).
pub fn confine(world: &mut dyn World, prefix: &Path, relative: &str) -> Option<PathBuf> {
    let relative = Path::new(relative);
    let mut out = prefix.to_path_buf();
    let mut depth = 0usize;
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                out.push(part);
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return None;
                }
                out.pop();
                depth -= 1;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    // The existing part of the path, resolved through links, stays inside the prefix.
    let root = world
        .canonical(prefix)
        .unwrap_or_else(|| prefix.to_path_buf());
    let mut existing = out.clone();
    loop {
        if let Some(resolved) = world.canonical(&existing) {
            return resolved.starts_with(&root).then_some(out);
        }
        if existing == prefix || !existing.pop() {
            return Some(out);
        }
    }
}

/// The keys of a key file: a TOML table of key names to base64 keys (DECISION D-103). The
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
}
