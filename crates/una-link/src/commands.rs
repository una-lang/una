//! The file: the Mach-O header, the load commands, the segments' bytes and `__LINKEDIT`
//! (`docs/plan/0124-step3-design.md` §6), then the UUID and the signature.

use sha2::{Digest, Sha256};

use crate::bytes::{self, Writer, align_up};
use crate::fixups::{self, ChainSegment};
use crate::gc::Target;
use crate::layout::{BASE, Image, SEGMENT_ALIGN};
use crate::problem::{Problem, Result};
use crate::sections::Seg;
use crate::signature::{self, Signed};

const LC_SEGMENT_64: u32 = 0x19;
const LC_SYMTAB: u32 = 0x2;
const LC_DYSYMTAB: u32 = 0xb;
const LC_LOAD_DYLIB: u32 = 0xc;
const LC_LOAD_DYLINKER: u32 = 0xe;
const LC_UUID: u32 = 0x1b;
const LC_CODE_SIGNATURE: u32 = 0x1d;
const LC_FUNCTION_STARTS: u32 = 0x26;
const LC_DATA_IN_CODE: u32 = 0x29;
const LC_BUILD_VERSION: u32 = 0x32;
const LC_MAIN: u32 = 0x8000_0028;
const LC_DYLD_EXPORTS_TRIE: u32 = 0x8000_0033;
const LC_DYLD_CHAINED_FIXUPS: u32 = 0x8000_0034;
const MH_NOUNDEFS: u32 = 0x1;
const MH_DYLDLINK: u32 = 0x4;
const MH_TWOLEVEL: u32 = 0x80;
const MH_PIE: u32 = 0x20_0000;
const MH_HAS_TLV_DESCRIPTORS: u32 = 0x80_0000;
const SG_READ_ONLY: u32 = 0x10;
const INDIRECT_SYMBOL_LOCAL: u32 = 0x8000_0000;
/// macOS 13.0.0, packed as `xxxx.yy.zz`.
const MACOS_13: u32 = 0x000d_0000;
const DYLINKER: &str = "/usr/lib/dyld";
const LIBSYSTEM: &str = "/usr/lib/libSystem.B.dylib";

/// The facts of the target that the commands name.
pub(crate) struct Target13 {
    /// libSystem's current version, packed.
    pub(crate) libsystem_version: u32,
}

/// One segment of the output.
struct Segment {
    name: &'static str,
    vmaddr: u64,
    vmsize: u64,
    fileoff: u64,
    filesize: u64,
    prot: u32,
    flags: u32,
    sections: Vec<usize>,
}

/// Writes the executable for `image`.
pub(crate) fn write(image: &Image<'_>, target: &Target13, signed: &Signed<'_>) -> Result<Vec<u8>> {
    // The segments.
    let mut segments: Vec<Segment> = Vec::new();
    let groups = [
        (Seg::Text, "__TEXT", 5u32, 0u32),
        (Seg::DataConst, "__DATA_CONST", 3, SG_READ_ONLY),
        (Seg::Data, "__DATA", 3, 0),
    ];
    let mut file_cursor = 0u64;
    let mut vm_cursor = BASE;
    for (seg, name, prot, flags) in groups {
        let members: Vec<usize> = image
            .sections
            .iter()
            .enumerate()
            .filter(|(_, s)| s.out.segment() == seg)
            .map(|(i, _)| i)
            .collect();
        if members.is_empty() {
            continue;
        }
        let start = if seg == Seg::Text { BASE } else { vm_cursor };
        let vm_end = members
            .iter()
            .filter_map(|i| image.sections.get(*i))
            .map(|s| s.addr + s.size)
            .max()
            .unwrap_or(start);
        let file_end = members
            .iter()
            .filter_map(|i| image.sections.get(*i))
            .filter(|s| !s.out.zerofill())
            .map(|s| s.addr + s.size)
            .max()
            .unwrap_or(start);
        let vmsize = align_up(vm_end - start, SEGMENT_ALIGN);
        let filesize = align_up(file_end.max(start) - start, SEGMENT_ALIGN);
        segments.push(Segment {
            name,
            vmaddr: start,
            vmsize,
            fileoff: file_cursor,
            filesize,
            prot,
            flags,
            sections: members,
        });
        file_cursor += filesize;
        vm_cursor = start + vmsize;
    }
    // Sections must lie at their segment's offsets: the layout gave each segment's start the
    // address its file offset implies.
    for segment in &segments {
        if segment.vmaddr - BASE != segment.fileoff && segment.name != "__TEXT" {
            return Err(Problem::internal(format!(
                "the segment {} does not start at the file offset its address implies",
                segment.name
            )));
        }
    }
    let linkedit_off = file_cursor;
    let linkedit_addr = vm_cursor;

    // __LINKEDIT, but for the fixups, which need the file's bytes, and the signature.
    let mut exports = Writer::default();
    exports.u8(0);
    exports.u8(0);
    exports.pad_to(8);
    let mut starts = Writer::default();
    let mut previous = BASE;
    for address in &image.function_starts {
        starts.uleb(address - previous);
        previous = *address;
    }
    starts.u8(0);
    starts.pad_to(8);
    let mut strings = Writer::default();
    strings.u8(b' ');
    strings.u8(0);
    let mut symtab = Writer::default();
    for symbol in &image.symbols {
        let offset = u32::try_from(strings.len())
            .map_err(|_| Problem::layout("a string table over 4 GiB"))?;
        strings.raw(symbol.name);
        strings.u8(0);
        symtab.u32(offset);
        symtab.u8(symbol.n_type);
        symtab.u8(symbol.n_sect);
        symtab.u16(symbol.n_desc);
        symtab.u64(symbol.value);
    }
    let (locals, externals) = image.symbol_counts;
    let first_import = locals + externals;
    let import_index = |name: &[u8]| -> Result<u32> {
        image
            .imports
            .iter()
            .position(|(n, _)| *n == name)
            .and_then(|p| u32::try_from(p).ok())
            .map(|p| first_import + p)
            .ok_or_else(|| Problem::internal("an indirect symbol that is not an import"))
    };
    let mut indirect = Writer::default();
    for name in &image.stubs {
        indirect.u32(import_index(name)?);
    }
    for slot in &image.got {
        match slot {
            Target::Import(name) => indirect.u32(import_index(name)?),
            _ => indirect.u32(INDIRECT_SYMBOL_LOCAL),
        }
    }
    indirect.pad_to(8);

    // The chained fixups' blob does not depend on the file's bytes, only on the fixups, so its
    // size is known before the file is assembled; it is computed on a scratch copy of the
    // segments' bytes and computed again on the file.
    let chain_segments: Vec<ChainSegment> = std::iter::once(ChainSegment {
        vmaddr: 0,
        vmsize: BASE,
        fileoff: 0,
        filesize: 0,
        writable: false,
    })
    .chain(segments.iter().map(|s| ChainSegment {
        vmaddr: s.vmaddr,
        vmsize: s.vmsize,
        fileoff: s.fileoff,
        filesize: s.filesize,
        writable: s.prot & 2 != 0,
    }))
    .chain(std::iter::once(ChainSegment {
        vmaddr: linkedit_addr,
        vmsize: 0,
        fileoff: linkedit_off,
        filesize: 0,
        writable: false,
    }))
    .collect();
    let mut file = vec![0u8; usize::try_from(linkedit_off).unwrap_or(0)];
    let chains = fixups::write(
        &mut file,
        &chain_segments,
        &image.fixups,
        &image.imports,
        BASE,
    )?;

    let mut linkedit = Writer::default();
    let chains_at = linkedit_off + linkedit.len();
    linkedit.raw(&chains);
    let exports_at = linkedit_off + linkedit.len();
    linkedit.raw(&exports.bytes);
    let starts_at = linkedit_off + linkedit.len();
    linkedit.raw(&starts.bytes);
    let data_in_code_at = linkedit_off + linkedit.len();
    let symtab_at = linkedit_off + linkedit.len();
    linkedit.raw(&symtab.bytes);
    let indirect_at = linkedit_off + linkedit.len();
    linkedit.raw(&indirect.bytes);
    let strings_at = linkedit_off + linkedit.len();
    // The string table ends where the signature starts, 16-aligned, as codesign expects.
    while !(strings_at + strings.len()).is_multiple_of(16) {
        strings.u8(0);
    }
    linkedit.raw(&strings.bytes);
    let signature_at = linkedit_off + linkedit.len();
    let signature_size = signature::size(signed, signature_at);
    let linkedit_size = linkedit.len() + signature_size;

    // The header and the load commands.
    let mut commands = Writer::default();
    let mut ncmds = 0u32;
    let mut segment_command = |w: &mut Writer,
                               name: &str,
                               vmaddr: u64,
                               vmsize: u64,
                               fileoff: u64,
                               filesize: u64,
                               prot: u32,
                               flags: u32,
                               sections: &[usize]| {
        ncmds += 1;
        w.u32(LC_SEGMENT_64);
        w.u32(72 + 80 * u32::try_from(sections.len()).unwrap_or(0));
        w.name16(name);
        w.u64(vmaddr);
        w.u64(vmsize);
        w.u64(fileoff);
        w.u64(filesize);
        w.u32(prot);
        w.u32(prot);
        w.u32(u32::try_from(sections.len()).unwrap_or(0));
        w.u32(flags);
    };
    segment_command(&mut commands, "__PAGEZERO", 0, BASE, 0, 0, 0, 0, &[]);
    for segment in &segments {
        segment_command(
            &mut commands,
            segment.name,
            segment.vmaddr,
            segment.vmsize,
            segment.fileoff,
            segment.filesize,
            segment.prot,
            segment.flags,
            &segment.sections,
        );
        for index in &segment.sections {
            let section = image
                .sections
                .get(*index)
                .ok_or_else(|| Problem::internal("a section index"))?;
            commands.name16(section.out.name());
            commands.name16(section.out.segment_name());
            commands.u64(section.addr);
            commands.u64(section.size);
            let offset = if section.out.zerofill() {
                0
            } else {
                u32::try_from(segment.fileoff + (section.addr - segment.vmaddr))
                    .map_err(|_| Problem::layout("a section beyond 4 GiB in the file"))?
            };
            commands.u32(offset);
            commands.u32(section.align);
            commands.u32(0);
            commands.u32(0);
            commands.u32(section.out.flags());
            commands.u32(section.reserved1);
            commands.u32(section.reserved2);
            commands.u32(0);
        }
    }
    segment_command(
        &mut commands,
        "__LINKEDIT",
        linkedit_addr,
        align_up(linkedit_size, SEGMENT_ALIGN),
        linkedit_off,
        linkedit_size,
        1,
        0,
        &[],
    );
    let data_command = |w: &mut Writer, cmd: u32, at: u64, size: u64| -> Result<()> {
        w.u32(cmd);
        w.u32(16);
        w.u32(u32::try_from(at).map_err(|_| Problem::layout("__LINKEDIT beyond 4 GiB"))?);
        w.u32(u32::try_from(size).map_err(|_| Problem::layout("__LINKEDIT beyond 4 GiB"))?);
        Ok(())
    };
    data_command(
        &mut commands,
        LC_DYLD_CHAINED_FIXUPS,
        chains_at,
        u64::try_from(chains.len()).unwrap_or(0),
    )?;
    data_command(
        &mut commands,
        LC_DYLD_EXPORTS_TRIE,
        exports_at,
        exports.len(),
    )?;
    let nsyms = u32::try_from(image.symbols.len()).unwrap_or(0);
    commands.u32(LC_SYMTAB);
    commands.u32(24);
    commands.u32(u32::try_from(symtab_at).unwrap_or(0));
    commands.u32(nsyms);
    commands.u32(u32::try_from(strings_at).unwrap_or(0));
    commands.u32(u32::try_from(strings.len()).unwrap_or(0));
    commands.u32(LC_DYSYMTAB);
    commands.u32(80);
    commands.u32(0);
    commands.u32(locals);
    commands.u32(locals);
    commands.u32(externals);
    commands.u32(first_import);
    commands.u32(nsyms - first_import);
    for _ in 0..6 {
        commands.u32(0); // toc, module table, external references
    }
    let nindirect = u32::try_from(image.stubs.len() + image.got.len()).unwrap_or(0);
    commands.u32(if nindirect > 0 {
        u32::try_from(indirect_at).unwrap_or(0)
    } else {
        0
    });
    commands.u32(nindirect);
    for _ in 0..4 {
        commands.u32(0); // external and local relocations
    }
    commands.u32(LC_LOAD_DYLINKER);
    commands.u32(32);
    commands.u32(12);
    commands.raw(DYLINKER.as_bytes());
    commands.u8(0);
    commands.pad_to(8);
    let uuid_at = 32 + commands.len() + 8;
    commands.u32(LC_UUID);
    commands.u32(24);
    commands.raw(&[0u8; 16]);
    commands.u32(LC_BUILD_VERSION);
    commands.u32(24);
    commands.u32(1); // PLATFORM_MACOS
    commands.u32(MACOS_13);
    commands.u32(MACOS_13);
    commands.u32(0);
    commands.u32(LC_MAIN);
    commands.u32(24);
    commands.u64(image.entry - BASE);
    commands.u64(0);
    commands.u32(LC_LOAD_DYLIB);
    commands.u32(56);
    commands.u32(24);
    commands.u32(2);
    commands.u32(target.libsystem_version);
    commands.u32(0x0001_0000);
    commands.raw(LIBSYSTEM.as_bytes());
    commands.u8(0);
    commands.pad_to(8);
    data_command(&mut commands, LC_FUNCTION_STARTS, starts_at, starts.len())?;
    data_command(&mut commands, LC_DATA_IN_CODE, data_in_code_at, 0)?;
    data_command(
        &mut commands,
        LC_CODE_SIGNATURE,
        signature_at,
        signature_size,
    )?;
    ncmds += 12;
    if 32 + commands.len() != image.header_size {
        return Err(Problem::internal(format!(
            "the load commands take {} bytes, not the {} the layout planned",
            32 + commands.len(),
            image.header_size
        )));
    }
    let mut header = Writer::default();
    header.u32(0xfeed_facf);
    header.u32(0x0100_000c); // CPU_TYPE_ARM64
    header.u32(0); // CPU_SUBTYPE_ARM64_ALL
    header.u32(2); // MH_EXECUTE
    header.u32(ncmds);
    header.u32(u32::try_from(commands.len()).unwrap_or(0));
    let mut flags = MH_NOUNDEFS | MH_DYLDLINK | MH_TWOLEVEL | MH_PIE;
    if image.tlv {
        flags |= MH_HAS_TLV_DESCRIPTORS;
    }
    header.u32(flags);
    header.u32(0);

    // The file: header, commands, sections, fixups, __LINKEDIT.
    bytes::put(&mut file, 0, &header.bytes)?;
    bytes::put(&mut file, 32, &commands.bytes)?;
    for segment in &segments {
        for index in &segment.sections {
            let section = image
                .sections
                .get(*index)
                .ok_or_else(|| Problem::internal("a section index"))?;
            if section.out.zerofill() {
                continue;
            }
            bytes::put(
                &mut file,
                segment.fileoff + (section.addr - segment.vmaddr),
                &section.bytes,
            )?;
        }
    }
    fixups::write(
        &mut file,
        &chain_segments,
        &image.fixups,
        &image.imports,
        BASE,
    )?;
    file.extend_from_slice(&linkedit.bytes);
    file.resize(
        usize::try_from(signature_at + signature_size).unwrap_or(0),
        0,
    );

    // The UUID: the digest of the file with the UUID and the signature zero.
    let digest = Sha256::digest(&file);
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(digest.get(..16).unwrap_or(&[0; 16]));
    if let Some(byte) = uuid.get_mut(6) {
        *byte = (*byte & 0x0f) | 0x40;
    }
    if let Some(byte) = uuid.get_mut(8) {
        *byte = (*byte & 0x3f) | 0x80;
    }
    bytes::put(&mut file, uuid_at, &uuid)?;
    let text = segments.first().map_or((0, 0), |s| (s.fileoff, s.filesize));
    let signed = Signed { text, ..*signed };
    let blob = signature::sign(&signed, &file, signature_at)?;
    bytes::put(&mut file, signature_at, &blob)?;
    Ok(file)
}
