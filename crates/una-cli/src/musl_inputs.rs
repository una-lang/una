//! The musl inputs built into a Linux `una` (ADR 0124 rules 12 and 20, step 4a,
//! `docs/plan/0124-step4-design.md` §2): musl's `libc.a`, the start-up objects of a static-pie
//! executable and `libunwind.a`, from the pinned Rust toolchain's `self-contained` directory of
//! the musl target of `una`'s own architecture, each with the digest that the build script
//! (`crates/una-cli/build.rs`) checked. Only the target of the `una` binary is carried, never
//! another one; a macOS `una` and a development build carry none.
//!
//! The internal linker of Linux (step 4, slices 4b and 4c) links them. Until it does, `una
//! build` on Linux links through the system `cc` as before, and checks these bytes against
//! their digests before each link, as it checks the runtime's, so that a damaged `una` fails
//! closed (U8001) and the build record names only what `una` really carries.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

/// One embedded input: its file name in the toolchain's directory, its bytes and its digest.
pub(crate) struct Input {
    /// The file name, such as `libc.a`.
    pub(crate) name: &'static str,
    bytes: &'static [u8],
    /// The SHA-256 the build checked, in hex.
    pub(crate) sha256: &'static str,
}

/// The name of the unwinder's archive, which the build record lists apart (rule 20).
const LIBUNWIND: &str = "libunwind.a";

/// One input embedded from the build script's output directory.
#[cfg(una_musl_embedded)]
macro_rules! input {
    ($name:literal, $variable:literal) => {
        Input {
            name: $name,
            bytes: include_bytes!(concat!(env!("OUT_DIR"), "/musl/", $name)),
            sha256: env!($variable),
        }
    };
}

/// The inputs, in the fixed order of the build script.
#[cfg(una_musl_embedded)]
static INPUTS: &[Input] = &[
    input!("libc.a", "UNA_EMBED_MUSL_SHA256_LIBC"),
    input!("rcrt1.o", "UNA_EMBED_MUSL_SHA256_RCRT1"),
    input!("crti.o", "UNA_EMBED_MUSL_SHA256_CRTI"),
    input!("crtn.o", "UNA_EMBED_MUSL_SHA256_CRTN"),
    input!("crtbeginS.o", "UNA_EMBED_MUSL_SHA256_CRTBEGINS"),
    input!("crtendS.o", "UNA_EMBED_MUSL_SHA256_CRTENDS"),
    input!("libunwind.a", "UNA_EMBED_MUSL_SHA256_LIBUNWIND"),
];

/// The inputs: none in a macOS `una` or a development build.
#[cfg(not(una_musl_embedded))]
static INPUTS: &[Input] = &[];

/// The digests of musl's own inputs, by file name, for the build record's `musl_sha256s`
/// (rule 20); empty when `una` carries none.
pub(crate) fn musl_sha256s() -> BTreeMap<&'static str, &'static str> {
    INPUTS
        .iter()
        .filter(|input| input.name != LIBUNWIND)
        .map(|input| (input.name, input.sha256))
        .collect()
}

/// The digest of `libunwind.a`, for the build record's `libunwind_sha256` (rule 20); empty when
/// `una` carries none.
pub(crate) fn libunwind_sha256() -> &'static str {
    INPUTS
        .iter()
        .find(|input| input.name == LIBUNWIND)
        .map_or("", |input| input.sha256)
}

/// Checks every embedded input against its digest. O(size of the inputs).
///
/// # Errors
///
/// The message of U8001 naming the first input that is missing or differs.
pub(crate) fn verify_all() -> Result<(), String> {
    INPUTS
        .iter()
        .try_for_each(|input| verify(input.name, input.bytes, input.sha256))
}

/// Checks `bytes`, the embedded input `name`, against `sha256`, the hex digest recorded at the
/// build. O(size of the input).
///
/// # Errors
///
/// The message of U8001 when `bytes` is empty or its digest differs.
fn verify(name: &str, bytes: &[u8], sha256: &str) -> Result<(), String> {
    if bytes.is_empty() {
        return Err(format!(
            "the musl input `{name}` built into `una` is missing; this `una` is damaged, so reinstall it"
        ));
    }
    let found = format!("{:x}", Sha256::digest(bytes));
    if found == sha256 {
        Ok(())
    } else {
        Err(format!(
            "the musl input `{name}` built into `una` does not match the digest recorded at its build (recorded {sha256}, found {found}); this `una` is damaged, so reinstall it"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_input_that_matches_its_digest_passes_and_a_damaged_one_fails_closed() {
        let bytes = b"!<arch>\n/ members".to_vec();
        let recorded = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(verify("libc.a", &bytes, &recorded), Ok(()));
        let missing = verify("libc.a", &[], &recorded).unwrap_err();
        assert!(
            missing.contains("`libc.a`") && missing.contains("is missing"),
            "{missing}"
        );
        let mut damaged = bytes;
        if let Some(byte) = damaged.last_mut() {
            *byte ^= 1;
        }
        let message = verify("rcrt1.o", &damaged, &recorded).unwrap_err();
        assert!(message.contains("`rcrt1.o`"), "{message}");
        assert!(message.contains(&recorded), "{message}");
    }

    #[test]
    fn the_record_lists_musl_s_inputs_apart_from_libunwind() {
        // A macOS `una` and a development build carry none (rule 12): both keys are empty; a
        // Linux release lists the six musl files and libunwind's digest.
        let musl = musl_sha256s();
        assert!(!musl.contains_key(LIBUNWIND));
        if cfg!(una_musl_embedded) {
            assert_eq!(
                musl.keys().copied().collect::<Vec<_>>(),
                [
                    "crtbeginS.o",
                    "crtendS.o",
                    "crti.o",
                    "crtn.o",
                    "libc.a",
                    "rcrt1.o"
                ]
            );
            assert_eq!(libunwind_sha256().len(), 64);
            assert_eq!(verify_all(), Ok(()));
        } else {
            assert!(musl.is_empty());
            assert_eq!(libunwind_sha256(), "");
            assert_eq!(verify_all(), Ok(()));
        }
    }
}
