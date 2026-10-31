//! The native runtime built into `una` (ADR 0124 rules 12 to 15, step 2,
//! `docs/plan/0124-step2-design.md` §2 and §3): the only place that names its bytes.
//!
//! A release `una` carries `libuna_rt.a`, built from the same commit and the same build, its
//! digest checked by the build script (`crates/una-cli/build.rs`) and recorded in the build
//! record and the report. `una build` checks the bytes against that digest again before each
//! link, so a damaged `una` fails closed with U8001 rather than linking a runtime that is not
//! the one it was built with; it then writes them into the link's private directory, which the
//! system `cc` reads until the internal linker of step 3 reads them in place. A development
//! build embeds nothing and reads the runtime from a file (rule 19, `crate::build`).

use std::borrow::Cow;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The runtime's bytes, embedded by a release build.
#[cfg(una_runtime_embedded)]
static BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libuna_rt.a"));

/// The SHA-256 of the embedded runtime, in hex, as the build script checked it; `None` in a
/// development build, which embeds no runtime.
#[cfg(una_runtime_embedded)]
pub(crate) const SHA256: Option<&str> = Some(env!("UNA_EMBED_RUNTIME_SHA256"));
/// The SHA-256 of the embedded runtime, in hex, as the build script checked it; `None` in a
/// development build, which embeds no runtime.
#[cfg(not(una_runtime_embedded))]
pub(crate) const SHA256: Option<&str> = None;

/// A runtime held by `una`: its bytes and the digest they must have.
pub(crate) struct Embedded {
    bytes: Cow<'static, [u8]>,
    sha256: Cow<'static, str>,
}

/// The runtime built into this `una`, `None` in a development build (rule 19).
///
/// In the toolchain's own test builds (the feature `test-hooks`, never in a shipped `una`), a
/// development build may stand a file in for it: `UNA_TEST_EMBEDDED_RUNTIME` names the file and
/// `UNA_TEST_EMBEDDED_RUNTIME_SHA256` the digest it must have, so that tests run the release
/// path, including a missing or damaged runtime. A file that cannot be read is held as no
/// bytes, which the digest check refuses. `scripts/check-release.sh` refuses a `una` with the
/// test hooks by their marker (`crate::solver_child`).
///
/// # Errors
///
/// None in a release or plain development build; a test hook without its digest.
pub(crate) fn embedded(environment: crate::Environment<'_>) -> Result<Option<Embedded>, String> {
    let _ = environment;
    #[cfg(una_runtime_embedded)]
    {
        Ok(SHA256.map(|sha256| Embedded {
            bytes: Cow::Borrowed(BYTES),
            sha256: Cow::Borrowed(sha256),
        }))
    }
    #[cfg(all(not(una_runtime_embedded), feature = "test-hooks"))]
    {
        let Some(path) = crate::variable(environment, "UNA_TEST_EMBEDDED_RUNTIME") else {
            return Ok(None);
        };
        let sha256 = crate::variable(environment, "UNA_TEST_EMBEDDED_RUNTIME_SHA256")
            .ok_or("UNA_TEST_EMBEDDED_RUNTIME needs UNA_TEST_EMBEDDED_RUNTIME_SHA256")?;
        Ok(Some(Embedded {
            bytes: Cow::Owned(std::fs::read(path).unwrap_or_default()),
            sha256: Cow::Owned(sha256),
        }))
    }
    #[cfg(all(not(una_runtime_embedded), not(feature = "test-hooks")))]
    {
        Ok(None)
    }
}

impl Embedded {
    /// The bytes, once they match the recorded digest.
    ///
    /// # Errors
    ///
    /// The message of U8001 when the bytes are missing or differ from the digest that the build
    /// checked: this `una` is damaged, and nothing is linked with them.
    pub(crate) fn verified(&self) -> Result<&[u8], String> {
        verify(&self.bytes, &self.sha256).map(|()| self.bytes.as_ref())
    }
}

/// Checks `bytes` against `sha256`, the hex digest recorded at the build. O(size of the runtime).
///
/// # Errors
///
/// The message of U8001 when `bytes` is empty (the runtime is missing) or its digest differs.
pub(crate) fn verify(bytes: &[u8], sha256: &str) -> Result<(), String> {
    if bytes.is_empty() {
        return Err(
            "the native runtime built into `una` is missing; this `una` is damaged, so reinstall it"
                .to_owned(),
        );
    }
    let found = format!("{:x}", Sha256::digest(bytes));
    if found == sha256 {
        Ok(())
    } else {
        Err(format!(
            "the native runtime built into `una` does not match the digest recorded at its build (recorded {sha256}, found {found}); this `una` is damaged, so reinstall it"
        ))
    }
}

/// The names the runtime may take in the link's private directory, the first one that is not
/// the object's or the executable's.
const NAMES: [&str; 2] = ["libuna_rt.a", "libuna_rt-embedded.a"];

/// Writes `bytes` into `private`, the link's private directory (mode 0700, fresh for this
/// build, #1160), as a new file of mode 0600 that no other file may already hold
/// (`create_new`, `O_EXCL`), under a name distinct from each of `taken`, and returns its path.
/// The file goes with the directory when the build ends. O(size of the runtime).
///
/// # Errors
///
/// Any error of the file system, including a file already at that name.
pub(crate) fn write_into(bytes: &[u8], private: &Path, taken: &[&Path]) -> io::Result<PathBuf> {
    let path = NAMES
        .iter()
        .map(|name| private.join(name))
        .find(|path| !taken.contains(&path.as_path()))
        .ok_or_else(|| io::Error::other("no free name for the runtime"))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    #[test]
    fn a_runtime_that_matches_its_digest_is_used() {
        let bytes = b"!<arch>\nmembers";
        assert_eq!(verify(bytes, &digest(bytes)), Ok(()));
    }

    #[test]
    fn a_missing_or_corrupted_runtime_fails_closed() {
        let bytes = b"!<arch>\nmembers".to_vec();
        let recorded = digest(&bytes);
        let missing = verify(&[], &recorded).unwrap_err();
        assert!(missing.contains("is missing"), "{missing}");
        let mut corrupted = bytes;
        if let Some(byte) = corrupted.last_mut() {
            *byte ^= 1;
        }
        let message = verify(&corrupted, &recorded).unwrap_err();
        assert!(message.contains("does not match the digest"), "{message}");
        assert!(message.contains(&recorded), "{message}");
        let held = Embedded {
            bytes: Cow::Owned(corrupted),
            sha256: Cow::Owned(recorded),
        };
        assert!(held.verified().is_err());
    }

    #[cfg(not(una_runtime_embedded))]
    #[test]
    fn a_development_build_embeds_no_runtime() {
        // The test builds are development builds: no runtime, no digest (rule 19).
        assert_eq!(SHA256, None);
        let environment = |_: &str| -> Result<Option<String>, una_interp::NotText> { Ok(None) };
        assert!(matches!(embedded(&environment), Ok(None)));
    }

    #[cfg(unix)]
    #[test]
    fn the_runtime_is_written_new_and_private_never_over_a_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("una-rt-embed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let written = write_into(b"runtime", &dir, &[]).unwrap();
        assert_eq!(written, dir.join("libuna_rt.a"));
        assert_eq!(std::fs::read(&written).unwrap(), b"runtime");
        let mode = std::fs::metadata(&written).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // `create_new`: a file already at the name, planted or left, is never written through.
        assert!(write_into(b"other", &dir, &[]).is_err());
        assert_eq!(std::fs::read(&written).unwrap(), b"runtime");
        // An executable named `libuna_rt.a` keeps its name; the runtime takes the other one.
        let produced = dir.join("libuna_rt.a");
        let other = write_into(b"runtime", &dir, &[&produced]).unwrap();
        assert_eq!(other, dir.join("libuna_rt-embedded.a"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
