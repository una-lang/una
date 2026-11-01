//! The verifier (ADR 0124 rule 34): every executable is read back, with readers of its own that
//! share nothing with the writer but the format, and refused (U8001) unless it holds the
//! hardening the record requires. It runs on every link; its cost is one pass over the output.
//!
//! Checked: the header names an arm64 position-independent executable whose stack is not
//! executable; no segment is both writable and executable, and `__DATA_CONST` is read-only after
//! the fixups; every section lies inside its segment and every segment inside the file; the load
//! commands are only those of rule 25, with libSystem as the one library; every chained fixup
//! lies 8-aligned inside its page of a writable segment, a rebase reaches an address of the image
//! and a bind an import, and every import is in libSystem's list; the UUID is the file's digest;
//! the signature is the file's last bytes, its code limit is its offset, and its page hashes
//! match the file.

use sha2::{Digest, Sha256};

use crate::bytes;
use crate::problem::{Problem, Result};

const MH_EXECUTE: u32 = 2;
const MH_PIE: u32 = 0x20_0000;
const MH_ALLOW_STACK_EXECUTION: u32 = 0x2_0000;
const VM_PROT_WRITE: u32 = 2;
const VM_PROT_EXECUTE: u32 = 4;
const SG_READ_ONLY: u32 = 0x10;

fn refuse(what: impl Into<String>) -> Problem {
    Problem::internal(format!(
        "the verifier refuses the executable: {}",
        what.into()
    ))
}

struct Segment {
    name: String,
    vmaddr: u64,
    vmsize: u64,
    fileoff: u64,
    filesize: u64,
    initprot: u32,
}

/// Verifies `file`, an executable that [`crate::link`] wrote, against rule 34, with `exports`
/// the names libSystem may provide.
///
/// # Errors
///
/// U8001 naming the first check that fails.
pub(crate) fn verify(file: &[u8], exports: &dyn Fn(&[u8]) -> bool) -> Result<()> {
    let word =
        |at: u64| bytes::u32_at(file, at).map_err(|_| refuse("a field lies outside the file"));
    let quad =
        |at: u64| bytes::u64_at(file, at).map_err(|_| refuse("a field lies outside the file"));
    let len = u64::try_from(file.len()).unwrap_or(u64::MAX);
    if word(0)? != 0xfeed_facf || word(4)? != 0x0100_000c || word(12)? != MH_EXECUTE {
        return Err(refuse("it is not an arm64 Mach-O executable"));
    }
    let flags = word(24)?;
    if flags & MH_ALLOW_STACK_EXECUTION != 0 {
        return Err(refuse("its stack is executable (MH_ALLOW_STACK_EXECUTION)"));
    }
    if flags & MH_PIE == 0 {
        return Err(refuse("it is not position-independent (MH_PIE)"));
    }
    let count = word(16)?;
    let commands_end = 32 + u64::from(word(20)?);
    let mut segments: Vec<Segment> = Vec::new();
    let mut signature: Option<(u64, u64)> = None;
    let mut uuid_at: Option<u64> = None;
    let mut chained: Option<(u64, u64)> = None;
    let mut dylibs = 0;
    let mut at = 32u64;
    for _ in 0..count {
        let (cmd, size) = (word(at)?, u64::from(word(at + 4)?));
        if size < 8 || at + size > commands_end {
            return Err(refuse("a load command runs past the load commands"));
        }
        match cmd {
            0x19 => {
                let name: Vec<u8> = file
                    .get(
                        usize::try_from(at + 8).unwrap_or(usize::MAX)
                            ..usize::try_from(at + 24).unwrap_or(usize::MAX),
                    )
                    .unwrap_or(&[])
                    .iter()
                    .copied()
                    .take_while(|b| *b != 0)
                    .collect();
                let name = String::from_utf8_lossy(&name).into_owned();
                let segment = Segment {
                    vmaddr: quad(at + 24)?,
                    vmsize: quad(at + 32)?,
                    fileoff: quad(at + 40)?,
                    filesize: quad(at + 48)?,
                    initprot: word(at + 60)?,
                    name,
                };
                let maxprot = word(at + 56)?;
                for prot in [maxprot, segment.initprot] {
                    if prot & VM_PROT_WRITE != 0 && prot & VM_PROT_EXECUTE != 0 {
                        return Err(refuse(format!(
                            "the segment {} is writable and executable",
                            segment.name
                        )));
                    }
                }
                if segment.name == "__DATA_CONST" && word(at + 68)? & SG_READ_ONLY == 0 {
                    return Err(refuse(
                        "__DATA_CONST is not read-only after the fixups (SG_READ_ONLY)",
                    ));
                }
                if segment.fileoff.saturating_add(segment.filesize) > len {
                    return Err(refuse(format!(
                        "the segment {} lies past the end of the file",
                        segment.name
                    )));
                }
                let sections = u64::from(word(at + 64)?);
                for index in 0..sections {
                    let section = at + 72 + 80 * index;
                    let addr = quad(section + 32)?;
                    let size = quad(section + 40)?;
                    if addr < segment.vmaddr
                        || addr.saturating_add(size) > segment.vmaddr.saturating_add(segment.vmsize)
                    {
                        return Err(refuse(format!(
                            "a section lies outside the segment {}",
                            segment.name
                        )));
                    }
                }
                segments.push(segment);
            }
            0x1d => signature = Some((u64::from(word(at + 8)?), u64::from(word(at + 12)?))),
            0x1b => uuid_at = Some(at + 8),
            0x8000_0034 => chained = Some((u64::from(word(at + 8)?), u64::from(word(at + 12)?))),
            0xc => {
                dylibs += 1;
                let name_at = at + u64::from(word(at + 8)?);
                let name: Vec<u8> = file
                    .get(
                        usize::try_from(name_at).unwrap_or(usize::MAX)
                            ..usize::try_from(at + size).unwrap_or(usize::MAX),
                    )
                    .unwrap_or(&[])
                    .iter()
                    .copied()
                    .take_while(|b| *b != 0)
                    .collect();
                if name != b"/usr/lib/libSystem.B.dylib" {
                    return Err(refuse(format!(
                        "it loads {}",
                        String::from_utf8_lossy(&name)
                    )));
                }
            }
            0x80000033 | 0x2 | 0xb | 0xe | 0x32 | 0x8000_0028 | 0x26 | 0x29 => {}
            other => return Err(refuse(format!("a load command {other:#x} outside rule 25"))),
        }
        at += size;
    }
    if dylibs != 1 {
        return Err(refuse("it does not load exactly libSystem"));
    }
    let (signature_at, signature_size) =
        signature.ok_or_else(|| refuse("it has no code signature"))?;
    if signature_at.saturating_add(signature_size) != len || !signature_at.is_multiple_of(16) {
        return Err(refuse(
            "the signature is not the file's last bytes, 16-aligned",
        ));
    }
    verify_fixups(
        file,
        &segments,
        chained.ok_or_else(|| refuse("it has no chained fixups"))?,
        exports,
    )?;
    // The UUID: the digest of the file with the UUID and the signature zero.
    let uuid_at = uuid_at.ok_or_else(|| refuse("it has no LC_UUID"))?;
    let mut copy = file.to_vec();
    bytes::put(&mut copy, uuid_at, &[0; 16])?;
    if let Some(tail) = copy.get_mut(usize::try_from(signature_at).unwrap_or(usize::MAX)..) {
        tail.fill(0);
    }
    let digest = Sha256::digest(&copy);
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(digest.get(..16).unwrap_or(&[0; 16]));
    if let Some(b) = uuid.get_mut(6) {
        *b = (*b & 0x0f) | 0x40;
    }
    if let Some(b) = uuid.get_mut(8) {
        *b = (*b & 0x3f) | 0x80;
    }
    if file.get(
        usize::try_from(uuid_at).unwrap_or(usize::MAX)
            ..usize::try_from(uuid_at + 16).unwrap_or(usize::MAX),
    ) != Some(&uuid[..])
    {
        return Err(refuse("the UUID is not the digest of the file"));
    }
    verify_signature(file, signature_at, &segments)
}

fn verify_fixups(
    file: &[u8],
    segments: &[Segment],
    (at, size): (u64, u64),
    exports: &dyn Fn(&[u8]) -> bool,
) -> Result<()> {
    let word = |offset: u64| {
        bytes::u32_at(file, at + offset)
            .map_err(|_| refuse("the chained fixups lie outside the file"))
    };
    let half = |offset: u64| {
        bytes::array::<2>(file, at + offset)
            .map(u16::from_le_bytes)
            .map_err(|_| refuse("the chained fixups lie outside the file"))
    };
    if word(0)? != 0 {
        return Err(refuse("chained fixups of a version other than 0"));
    }
    let (starts, imports, symbols, imports_count) = (
        u64::from(word(4)?),
        u64::from(word(8)?),
        u64::from(word(12)?),
        u64::from(word(16)?),
    );
    if word(20)? != 1 || word(24)? != 0 {
        return Err(refuse(
            "chained fixups whose imports are not DYLD_CHAINED_IMPORT",
        ));
    }
    if starts >= size || imports > size || symbols > size {
        return Err(refuse("chained fixups whose tables lie outside their data"));
    }
    for index in 0..imports_count {
        let import = word(imports + 4 * index)?;
        if import & 0xff != 1 {
            return Err(refuse("an import from a library other than libSystem"));
        }
        let name_at = at + symbols + u64::from(import >> 9);
        let name: Vec<u8> = file
            .get(usize::try_from(name_at).unwrap_or(usize::MAX)..)
            .unwrap_or(&[])
            .iter()
            .copied()
            .take_while(|b| *b != 0)
            .collect();
        if !exports(&name) {
            return Err(refuse(format!(
                "the import `{}` is not in libSystem's export list",
                String::from_utf8_lossy(&name)
            )));
        }
    }
    let image_end = segments
        .iter()
        .map(|s| s.vmaddr.saturating_add(s.vmsize))
        .max()
        .unwrap_or(0);
    let image_start = segments
        .iter()
        .filter(|s| s.vmaddr > 0)
        .map(|s| s.vmaddr)
        .min()
        .unwrap_or(0);
    let seg_count = u64::from(word(starts)?);
    if seg_count != u64::try_from(segments.len()).unwrap_or(0) {
        return Err(refuse(
            "the chained fixups describe another number of segments",
        ));
    }
    for (index, segment) in segments.iter().enumerate() {
        let info = u64::from(word(starts + 4 + 4 * u64::try_from(index).unwrap_or(0))?);
        if info == 0 {
            continue;
        }
        let base = starts + info;
        let page_size = u64::from(half(base + 4)?);
        let format = half(base + 6)?;
        if page_size != 0x1000 || format != 2 {
            return Err(refuse(
                "chained fixups of another page size or pointer format",
            ));
        }
        if segment.initprot & VM_PROT_WRITE == 0 {
            return Err(refuse(format!(
                "chained fixups in the read-only segment {}",
                segment.name
            )));
        }
        let pages = u64::from(half(base + 20)?);
        for page in 0..pages {
            let start = half(base + 22 + 2 * page)?;
            if start == 0xffff {
                continue;
            }
            let page_at = segment.fileoff + page * page_size;
            let mut place = page_at + u64::from(start);
            loop {
                if !place.is_multiple_of(8)
                    || place + 8 > page_at + page_size
                    || place + 8 > segment.fileoff + segment.filesize
                {
                    return Err(refuse("a chained fixup outside its page or not 8-aligned"));
                }
                let value = bytes::u64_at(file, place)
                    .map_err(|_| refuse("a chained fixup outside the file"))?;
                if value >> 63 == 1 {
                    if value & 0xff_ffff >= imports_count {
                        return Err(refuse("a bind to an import that does not exist"));
                    }
                } else {
                    let target = value & ((1 << 36) - 1);
                    if target < image_start || target >= image_end {
                        return Err(refuse("a rebase to an address outside the image"));
                    }
                }
                let next = (value >> 51) & 0xfff;
                if next == 0 {
                    break;
                }
                place += next * 4;
            }
        }
    }
    Ok(())
}

fn verify_signature(file: &[u8], signature_at: u64, segments: &[Segment]) -> Result<()> {
    let be = |offset: u64| {
        bytes::array::<4>(file, signature_at + offset)
            .map(u32::from_be_bytes)
            .map_err(|_| refuse("the signature lies outside the file"))
    };
    if be(0)? != 0xfade_0cc0 {
        return Err(refuse("the signature is not a SuperBlob"));
    }
    let count = u64::from(be(8)?);
    let mut directory = None;
    for index in 0..count {
        if be(12 + 8 * index)? == 0 {
            directory = Some(u64::from(be(16 + 8 * index)?));
        }
    }
    let cd = directory.ok_or_else(|| refuse("the signature has no CodeDirectory"))?;
    if be(cd)? != 0xfade_0c02 || be(cd + 8)? != 0x20500 {
        return Err(refuse("the CodeDirectory is not of version 0x20500"));
    }
    let flags = be(cd + 12)?;
    if flags & 0x2 == 0 || flags & 0x1_0000 == 0 {
        return Err(refuse(
            "the signature is not ad-hoc with the hardened runtime",
        ));
    }
    let hash_offset = u64::from(be(cd + 16)?);
    let code_slots = u64::from(be(cd + 28)?);
    let code_limit = u64::from(be(cd + 32)?);
    let [hash_size, hash_type, _, page_shift] = bytes::array::<4>(file, signature_at + cd + 36)?;
    if code_limit != signature_at {
        return Err(refuse("the signature's code limit is not its offset"));
    }
    if hash_size != 32 || hash_type != 2 || page_shift != 12 {
        return Err(refuse(
            "the signature does not hash 4 KiB pages with SHA-256",
        ));
    }
    if code_slots != code_limit.div_ceil(4096) {
        return Err(refuse("the signature does not hash every page"));
    }
    let text_size = segments
        .iter()
        .find(|s| s.name == "__TEXT")
        .map_or(0, |s| s.filesize);
    let exec_limit = bytes::array::<8>(file, signature_at + cd + 72).map(u64::from_be_bytes)?;
    if exec_limit != text_size {
        return Err(refuse("the signature's executable segment is not __TEXT"));
    }
    let code = file
        .get(..usize::try_from(code_limit).unwrap_or(usize::MAX))
        .unwrap_or(&[]);
    for (index, page) in code.chunks(4096).enumerate() {
        let slot = signature_at + cd + hash_offset + 32 * u64::try_from(index).unwrap_or(0);
        let recorded = bytes::array::<32>(file, slot)
            .map_err(|_| refuse("a page hash lies outside the file"))?;
        if Sha256::digest(page).as_slice() != recorded {
            return Err(refuse(format!(
                "the hash of page {index} does not match the file"
            )));
        }
    }
    Ok(())
}
