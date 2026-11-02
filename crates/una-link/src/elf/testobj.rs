//! A writer of small ELF relocatable objects for the unit tests of the reader and the engine:
//! any section, symbol and relocation, including ones the reader must refuse.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

/// A section: relocations are (offset, symbol index, type, addend).
#[derive(Clone)]
pub(crate) struct Sec {
    pub(crate) name: &'static str,
    pub(crate) kind: u32,
    pub(crate) flags: u64,
    pub(crate) align: u64,
    pub(crate) data: Vec<u8>,
    /// The size of a `SHT_NOBITS` section (else the data's length).
    pub(crate) size: u64,
    pub(crate) relocs: Vec<(u64, u32, u32, i64)>,
}

/// A symbol: `section` is the 1-based index of its section, or `SHN_ABS` (0xfff1), `SHN_COMMON`
/// (0xfff2) or 0 (undefined).
#[derive(Clone)]
pub(crate) struct Sym {
    pub(crate) name: &'static str,
    pub(crate) info: u8,
    pub(crate) section: u16,
    pub(crate) value: u64,
    pub(crate) size: u64,
}

pub(crate) const WRITE: u64 = 0x1;
pub(crate) const ALLOC: u64 = 0x2;
pub(crate) const EXEC: u64 = 0x4;
pub(crate) const TLS: u64 = 0x400;
pub(crate) const ABS: u16 = 0xfff1;
pub(crate) const COMMON: u16 = 0xfff2;
pub(crate) const LOCAL: u8 = 0x00;
pub(crate) const GLOBAL_FUNC: u8 = 0x12;
pub(crate) const GLOBAL_OBJECT: u8 = 0x11;
pub(crate) const GLOBAL_NOTYPE: u8 = 0x10;
pub(crate) const GLOBAL_TLS: u8 = 0x16;
pub(crate) const X86_64: u16 = 62;
pub(crate) const AARCH64: u16 = 183;

/// A loaded section of `data` with `flags`.
pub(crate) fn sec(name: &'static str, flags: u64, data: Vec<u8>) -> Sec {
    let size = data.len() as u64;
    Sec {
        name,
        kind: 1,
        flags: flags | ALLOC,
        align: 16,
        data,
        size,
        relocs: Vec::new(),
    }
}

/// A symbol of `section` at `value`, of size 0.
pub(crate) fn sym(name: &'static str, info: u8, section: u16, value: u64) -> Sym {
    Sym {
        name,
        info,
        section,
        value,
        size: 0,
    }
}

/// The object: header, the sections, a `.rela` per section with relocations, `.symtab`,
/// `.strtab`, `.shstrtab`. Symbol `i` of `symbols` has index `i + 1`; locals must come first.
pub(crate) fn object(machine: u16, sections: &[Sec], symbols: &[Sym]) -> Vec<u8> {
    let mut shstrtab = vec![0u8];
    let mut name_of = |name: &str| {
        let at = u32::try_from(shstrtab.len()).unwrap();
        shstrtab.extend_from_slice(name.as_bytes());
        shstrtab.push(0);
        at
    };
    let mut strtab = vec![0u8];
    let mut symtab = vec![0u8; 24];
    for symbol in symbols {
        let at = u32::try_from(strtab.len()).unwrap();
        strtab.extend_from_slice(symbol.name.as_bytes());
        strtab.push(0);
        symtab.extend_from_slice(&at.to_le_bytes());
        symtab.push(symbol.info);
        symtab.push(0);
        symtab.extend_from_slice(&symbol.section.to_le_bytes());
        symtab.extend_from_slice(&symbol.value.to_le_bytes());
        symtab.extend_from_slice(&symbol.size.to_le_bytes());
    }
    let locals = 1 + symbols.iter().filter(|s| s.info >> 4 == 0).count();
    let mut headers: Vec<[u64; 8]> = vec![[0; 8]];
    let mut body: Vec<u8> = Vec::new();
    let base = 64u64;
    let place = |body: &mut Vec<u8>, bytes: &[u8]| {
        while !body.len().is_multiple_of(16) {
            body.push(0);
        }
        let at = base + body.len() as u64;
        body.extend_from_slice(bytes);
        at
    };
    let user = sections.len() as u64;
    let rela_count = sections.iter().filter(|s| !s.relocs.is_empty()).count() as u64;
    let symtab_index = 1 + user + rela_count;
    for s in sections {
        let at = if s.kind == 8 {
            place(&mut body, &[])
        } else {
            place(&mut body, &s.data)
        };
        let name = u64::from(name_of(s.name));
        let size = if s.kind == 8 {
            s.size
        } else {
            s.data.len() as u64
        };
        headers.push([
            name | (u64::from(s.kind) << 32),
            s.flags,
            0,
            at,
            size,
            0,
            s.align,
            0,
        ]);
    }
    for (index, s) in sections.iter().enumerate() {
        if s.relocs.is_empty() {
            continue;
        }
        let mut rela = Vec::new();
        for &(offset, symbol, kind, addend) in &s.relocs {
            rela.extend_from_slice(&offset.to_le_bytes());
            rela.extend_from_slice(&((u64::from(symbol) << 32) | u64::from(kind)).to_le_bytes());
            rela.extend_from_slice(&addend.to_le_bytes());
        }
        let at = place(&mut body, &rela);
        let name = u64::from(name_of(&format!(".rela{}", s.name)));
        let info = index as u64 + 1;
        headers.push([
            name | (4 << 32),
            0x40,
            0,
            at,
            rela.len() as u64,
            symtab_index | (info << 32),
            8,
            24,
        ]);
    }
    let at = place(&mut body, &symtab);
    let name = u64::from(name_of(".symtab"));
    headers.push([
        name | (2 << 32),
        0,
        0,
        at,
        symtab.len() as u64,
        (symtab_index + 1) | ((locals as u64) << 32),
        8,
        24,
    ]);
    let at = place(&mut body, &strtab);
    let name = u64::from(name_of(".strtab"));
    headers.push([name | (3 << 32), 0, 0, at, strtab.len() as u64, 0, 1, 0]);
    let name = u64::from(name_of(".shstrtab"));
    let shstrtab_bytes = shstrtab.clone();
    let at = place(&mut body, &shstrtab_bytes);
    headers.push([
        name | (3 << 32),
        0,
        0,
        at,
        shstrtab_bytes.len() as u64,
        0,
        1,
        0,
    ]);
    while !body.len().is_multiple_of(8) {
        body.push(0);
    }
    let shoff = base + body.len() as u64;
    let mut out = Vec::new();
    out.extend_from_slice(b"\x7fELF\x02\x01\x01\x00");
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&machine.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&shoff.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&64u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&64u16.to_le_bytes());
    out.extend_from_slice(&u16::try_from(headers.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&u16::try_from(headers.len() - 1).unwrap().to_le_bytes());
    out.extend_from_slice(&body);
    for [
        name_type,
        flags,
        addr,
        offset,
        size,
        link_info,
        align,
        entsize,
    ] in headers
    {
        out.extend_from_slice(
            &u32::try_from(name_type & 0xffff_ffff)
                .unwrap()
                .to_le_bytes(),
        );
        out.extend_from_slice(&u32::try_from(name_type >> 32).unwrap().to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&addr.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(link_info & 0xffff_ffff)
                .unwrap()
                .to_le_bytes(),
        );
        out.extend_from_slice(&u32::try_from(link_info >> 32).unwrap().to_le_bytes());
        out.extend_from_slice(&align.to_le_bytes());
        out.extend_from_slice(&entsize.to_le_bytes());
    }
    out
}

/// The offset of section header `index` in `bytes`.
pub(crate) fn header(bytes: &[u8], index: usize) -> usize {
    let shoff = u64::from_le_bytes(bytes[40..48].try_into().unwrap());
    usize::try_from(shoff).unwrap() + index * 64
}
