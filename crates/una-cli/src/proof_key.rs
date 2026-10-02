//! The user's secret key of the proof cache (ADR 0064): the file `UNA_PROOF_KEY` names, else
//! `$HOME/.una/proof-cache.key`, made on first use with 32 random bytes and readable by the
//! user only. Each entry of a package's cache carries a tag under this key, so the cache leaves
//! the Trusted zone and the key joins it instead.
//!
//! When there is no key to read or make, the cache is off: every obligation is solved, and the
//! build report's `tcb.proof_cache_key` is `null`. That is never an error.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::Environment;

/// The variable that names the key file instead of the default.
const OVERRIDE: &str = "UNA_PROOF_KEY";

/// The default key file, under the home directory, as the report shows it.
const DEFAULT_SHOWN: &str = "~/.una/proof-cache.key";

/// The bytes of a key made on first use.
const LENGTH: usize = 32;

/// A key shorter than this is not used.
const MINIMUM: usize = 16;

/// The user's key: its bytes, and its file as the build report shows it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProofKey {
    pub(crate) secret: Vec<u8>,
    pub(crate) shown: String,
}

impl std::fmt::Debug for ProofKey {
    // The secret never appears in output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProofKey")
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

/// The key, read or made, or `None` when there is none to use.
pub(crate) fn resolve(environment: Environment<'_>) -> Option<ProofKey> {
    let (path, shown) = match environment(OVERRIDE).filter(|value| !value.is_empty()) {
        Some(file) => (PathBuf::from(&file), file),
        None => {
            let home = environment("HOME").filter(|home| !home.is_empty())?;
            let path = Path::new(&home).join(".una").join("proof-cache.key");
            (path, DEFAULT_SHOWN.to_owned())
        }
    };
    let secret = read_or_make(&path)?;
    (secret.len() >= MINIMUM).then_some(ProofKey { secret, shown })
}

/// The bytes of the key file at `path`, made first when it does not exist. Two processes that
/// make it at once agree: the file is created only when absent, and the loser reads the
/// winner's.
fn read_or_make(path: &Path) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => return Some(bytes),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return None,
        Err(_) => {}
    }
    let mut secret = vec![0u8; LENGTH];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut secret))
        .ok()?;
    if let Some(dir) = path.parent() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir).ok()?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(&secret).ok()?;
            Some(secret)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => std::fs::read(path).ok(),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_made_once_read_after_and_never_shown() {
        let dir = std::env::temp_dir().join(format!("una-proof-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = dir.to_str().unwrap().to_owned();
        let environment = move |name: &str| (name == "HOME").then(|| home.clone());
        let made = resolve(&environment).unwrap();
        assert_eq!(made.secret.len(), LENGTH);
        assert_eq!(made.shown, "~/.una/proof-cache.key");
        let file = dir.join(".una").join("proof-cache.key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(resolve(&environment).unwrap(), made);
        assert!(!format!("{made:?}").contains(&format!("{:?}", made.secret)));
        // The override names another file; a key too short, or no home, is no key.
        let other = dir.join("other.key");
        std::fs::write(&other, b"short").unwrap();
        let other_name = other.to_str().unwrap().to_owned();
        let overriding = move |name: &str| (name == OVERRIDE).then(|| other_name.clone());
        assert_eq!(resolve(&overriding), None);
        assert_eq!(resolve(&|_: &str| None), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
