//! The ad-hoc signature (ADR 0124 rule 27, `docs/plan/0124-step3-design.md` §9): a SuperBlob with
//! one CodeDirectory of version 0x20500 (`adhoc | linker-signed | runtime`, SHA-256 hashes of 4
//! KiB pages up to the signature, the executable segment of a main binary) and, with
//! `--debuggable`, an entitlements blob with `com.apple.security.get-task-allow`. There is no
//! requirements slot, no team identifier and no CMS blob. The fields are big-endian.

use sha2::{Digest, Sha256};

use crate::problem::{Problem, Result};

const SUPERBLOB: u32 = 0xfade_0cc0;
const CODE_DIRECTORY: u32 = 0xfade_0c02;
const ENTITLEMENTS: u32 = 0xfade_7171;
const CS_ADHOC: u32 = 0x2;
const CS_RUNTIME: u32 = 0x1_0000;
const CS_LINKER_SIGNED: u32 = 0x2_0000;
const CS_EXECSEG_MAIN_BINARY: u64 = 0x1;
/// The hardened runtime's version field: macOS 13.0.0.
const RUNTIME_VERSION: u32 = 0x000d_0000;
const PAGE_SHIFT: u8 = 12;
const PAGE: usize = 1 << PAGE_SHIFT;
const HASH: usize = 32;
const HEADER: usize = 96;

/// The entitlements of a `--debuggable` executable.
const DEBUGGABLE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>com.apple.security.get-task-allow</key>\n\t<true/>\n</dict>\n</plist>\n";

/// What the signature covers.
pub(crate) struct Signed<'a> {
    /// The identifier: the output file's name, without its directory.
    pub(crate) identifier: &'a str,
    /// Whether the entitlements blob is present.
    pub(crate) debuggable: bool,
    /// The file offset of `__TEXT` and its size in the file.
    pub(crate) text: (u64, u64),
}

/// The signature's size for a file whose signature starts at `code_limit`.
pub(crate) fn size(signed: &Signed<'_>, code_limit: u64) -> u64 {
    let (count, entitlements) = if signed.debuggable {
        (2u64, 8 + DEBUGGABLE.len() as u64)
    } else {
        (1, 0)
    };
    let special = if signed.debuggable { 5u64 } else { 0 };
    let pages = code_limit.div_ceil(PAGE as u64);
    let directory =
        HEADER as u64 + signed.identifier.len() as u64 + 1 + (special + pages) * HASH as u64;
    let total = 12 + 8 * count + directory + entitlements;
    total.div_ceil(16) * 16
}

/// The signature of `file[..code_limit]`, of exactly [`size`] bytes.
///
/// # Errors
///
/// U8001 when the identifier holds a NUL or the file is too large for a 32-bit code limit.
pub(crate) fn sign(signed: &Signed<'_>, file: &[u8], code_limit: u64) -> Result<Vec<u8>> {
    if signed.identifier.as_bytes().contains(&0) || signed.identifier.is_empty() {
        return Err(Problem::internal(
            "the signature's identifier is empty or holds a NUL",
        ));
    }
    let limit = u32::try_from(code_limit)
        .map_err(|_| Problem::layout("an executable larger than 4 GiB cannot be signed here"))?;
    let code = file
        .get(..usize::try_from(code_limit).unwrap_or(usize::MAX))
        .ok_or_else(|| Problem::internal("the code limit lies past the file"))?;
    let entitlements: Vec<u8> = if signed.debuggable {
        let mut blob = Vec::new();
        blob.extend_from_slice(&ENTITLEMENTS.to_be_bytes());
        blob.extend_from_slice(
            &u32::try_from(8 + DEBUGGABLE.len())
                .unwrap_or(0)
                .to_be_bytes(),
        );
        blob.extend_from_slice(DEBUGGABLE.as_bytes());
        blob
    } else {
        Vec::new()
    };
    let special: u32 = if signed.debuggable { 5 } else { 0 };
    let pages: Vec<&[u8]> = code.chunks(PAGE).collect();
    let code_slots = u32::try_from(pages.len()).map_err(|_| Problem::layout("too many pages"))?;
    let ident_offset = HEADER as u32;
    let hash_offset = ident_offset
        + u32::try_from(signed.identifier.len()).unwrap_or(0)
        + 1
        + special * HASH as u32;
    let directory_len = hash_offset + code_slots * HASH as u32;
    let mut cd = Vec::with_capacity(directory_len as usize);
    let be32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_be_bytes());
    let be64 = |v: &mut Vec<u8>, x: u64| v.extend_from_slice(&x.to_be_bytes());
    be32(&mut cd, CODE_DIRECTORY);
    be32(&mut cd, directory_len);
    be32(&mut cd, 0x20500);
    be32(&mut cd, CS_ADHOC | CS_LINKER_SIGNED | CS_RUNTIME);
    be32(&mut cd, hash_offset);
    be32(&mut cd, ident_offset);
    be32(&mut cd, special);
    be32(&mut cd, code_slots);
    be32(&mut cd, limit);
    cd.push(HASH as u8);
    cd.push(2); // CS_HASHTYPE_SHA256
    cd.push(0); // platform
    cd.push(PAGE_SHIFT);
    be32(&mut cd, 0); // spare2
    be32(&mut cd, 0); // scatterOffset
    be32(&mut cd, 0); // teamOffset
    be32(&mut cd, 0); // spare3
    be64(&mut cd, 0); // codeLimit64
    be64(&mut cd, signed.text.0); // execSegBase
    be64(&mut cd, signed.text.1); // execSegLimit
    be64(&mut cd, CS_EXECSEG_MAIN_BINARY); // execSegFlags
    be32(&mut cd, RUNTIME_VERSION);
    be32(&mut cd, 0); // preEncryptOffset
    if cd.len() != HEADER {
        return Err(Problem::internal(
            "the CodeDirectory header has the wrong size",
        ));
    }
    cd.extend_from_slice(signed.identifier.as_bytes());
    cd.push(0);
    if signed.debuggable {
        // Slots -5 (entitlements) to -1, in that order before the code slots.
        cd.extend_from_slice(&Sha256::digest(&entitlements));
        cd.extend_from_slice(&[0u8; HASH * 4]);
    }
    for page in pages {
        cd.extend_from_slice(&Sha256::digest(page));
    }
    let count: u32 = if signed.debuggable { 2 } else { 1 };
    let mut blob = Vec::new();
    let directory_at = 12 + 8 * count;
    be32(&mut blob, SUPERBLOB);
    be32(&mut blob, 0);
    be32(&mut blob, count);
    be32(&mut blob, 0); // CSSLOT_CODEDIRECTORY
    be32(&mut blob, directory_at);
    if signed.debuggable {
        be32(&mut blob, 5); // CSSLOT_ENTITLEMENTS
        be32(&mut blob, directory_at + directory_len);
    }
    blob.extend_from_slice(&cd);
    blob.extend_from_slice(&entitlements);
    let length = u32::try_from(blob.len()).unwrap_or(0);
    if let Some(field) = blob.get_mut(4..8) {
        field.copy_from_slice(&length.to_be_bytes());
    }
    let padded = size(signed, code_limit);
    blob.resize(usize::try_from(padded).unwrap_or(0), 0);
    Ok(blob)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_is_one_code_directory_and_entitlements_only_when_debuggable() {
        let file = vec![7u8; 5000];
        let plain = Signed {
            identifier: "hello",
            debuggable: false,
            text: (0, 4096),
        };
        let blob = sign(&plain, &file, 5000).unwrap();
        assert_eq!(blob.len() as u64, size(&plain, 5000));
        assert_eq!(&blob[0..4], &SUPERBLOB.to_be_bytes());
        assert_eq!(&blob[8..12], &1u32.to_be_bytes());
        let cd = &blob[20..];
        assert_eq!(&cd[0..4], &CODE_DIRECTORY.to_be_bytes());
        assert_eq!(&cd[8..12], &0x20500u32.to_be_bytes());
        assert_eq!(&cd[12..16], &0x0003_0002u32.to_be_bytes());
        assert_eq!(&cd[28..32], &2u32.to_be_bytes(), "two code pages");
        let hash_offset = u32::from_be_bytes(cd[16..20].try_into().unwrap()) as usize;
        assert_eq!(
            &cd[hash_offset..hash_offset + 32],
            Sha256::digest(&file[..4096]).as_slice()
        );
        assert_eq!(&cd[96..102], b"hello\0");
        let debuggable = Signed {
            identifier: "hello",
            debuggable: true,
            text: (0, 4096),
        };
        let blob = sign(&debuggable, &file, 5000).unwrap();
        assert_eq!(&blob[8..12], &2u32.to_be_bytes());
        assert!(
            blob.windows(33)
                .any(|w| w == b"com.apple.security.get-task-allow")
        );
        assert!(
            sign(
                &Signed {
                    identifier: "",
                    debuggable: false,
                    text: (0, 0)
                },
                &file,
                10
            )
            .is_err()
        );
    }
}
