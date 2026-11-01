//! Hand-made inputs for the linker's tests: a writer of small arm64 Mach-O relocatable objects
//! and of `ar` archives with a symbol table, so each test states its input exactly.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::expect_used
)]

/// `ARM64_RELOC_*` types.
pub const UNSIGNED: u8 = 0;
pub const SUBTRACTOR: u8 = 1;
pub const BRANCH26: u8 = 2;
pub const PAGE21: u8 = 3;
pub const PAGEOFF12: u8 = 4;
pub const GOT_LOAD_PAGE21: u8 = 5;
pub const GOT_LOAD_PAGEOFF12: u8 = 6;
pub const ADDEND: u8 = 10;

/// A relocation as `relocation_info` writes it.
#[derive(Clone)]
pub struct Reloc {
    pub address: u32,
    pub symbolnum: u32,
    pub pcrel: bool,
    pub length: u8,
    pub external: bool,
    pub kind: u8,
}

/// A relocation against symbol `symbol` at `address`.
pub fn reloc(address: u32, symbol: u32, kind: u8) -> Reloc {
    let (pcrel, length) = match kind {
        BRANCH26 | PAGE21 | GOT_LOAD_PAGE21 => (true, 2),
        PAGEOFF12 | GOT_LOAD_PAGEOFF12 => (false, 2),
        _ => (false, 3),
    };
    Reloc {
        address,
        symbolnum: symbol,
        pcrel,
        length,
        external: true,
        kind,
    }
}

/// A section.
#[derive(Clone)]
pub struct Section {
    pub segment: &'static str,
    pub name: &'static str,
    pub flags: u32,
    pub align: u32,
    pub data: Vec<u8>,
    /// For a zero-fill section, its size; `data` is then empty.
    pub zerofill: u64,
    pub relocs: Vec<Reloc>,
}

/// `__TEXT,__text` with these instructions.
pub fn text(code: &[u32]) -> Section {
    Section {
        segment: "__TEXT",
        name: "__text",
        flags: 0x8000_0400,
        align: 2,
        data: code.iter().flat_map(|w| w.to_le_bytes()).collect(),
        zerofill: 0,
        relocs: Vec::new(),
    }
}

/// A data section of `segment,name` with these bytes.
pub fn data(segment: &'static str, name: &'static str, flags: u32, bytes: &[u8]) -> Section {
    Section {
        segment,
        name,
        flags,
        align: 3,
        data: bytes.to_vec(),
        zerofill: 0,
        relocs: Vec::new(),
    }
}

/// A symbol.
#[derive(Clone)]
pub struct Symbol {
    pub name: String,
    pub n_type: u8,
    pub n_sect: u8,
    pub n_desc: u16,
    /// The offset in its section; the writer adds the section's address.
    pub offset: u64,
}

/// An external symbol defined at `offset` of section `section` (1-based).
pub fn global(name: &str, section: u8, offset: u64) -> Symbol {
    Symbol {
        name: name.to_owned(),
        n_type: 0x0f,
        n_sect: section,
        n_desc: 0,
        offset,
    }
}

/// A local symbol defined at `offset` of section `section`.
pub fn local(name: &str, section: u8, offset: u64) -> Symbol {
    Symbol {
        name: name.to_owned(),
        n_type: 0x0e,
        n_sect: section,
        n_desc: 0,
        offset,
    }
}

/// An undefined external symbol.
pub fn undefined(name: &str) -> Symbol {
    Symbol {
        name: name.to_owned(),
        n_type: 0x01,
        n_sect: 0,
        n_desc: 0,
        offset: 0,
    }
}

/// An object.
#[derive(Clone, Default)]
pub struct Object {
    pub sections: Vec<Section>,
    pub symbols: Vec<Symbol>,
}

impl Object {
    /// The object's bytes: header, one segment command, a symtab command, then the sections'
    /// bytes, their relocations, the symbols and the strings.
    pub fn bytes(&self) -> Vec<u8> {
        let nsects = self.sections.len() as u32;
        let sizeofcmds = 72 + 80 * nsects + 24;
        let mut addrs = Vec::new();
        let mut addr = 0u64;
        for s in &self.sections {
            let a = 1u64 << s.align;
            addr = addr.div_ceil(a) * a;
            addrs.push(addr);
            addr += if s.zerofill > 0 {
                s.zerofill
            } else {
                s.data.len() as u64
            };
        }
        let vmsize = addr;
        let mut offset = 32 + sizeofcmds as u64;
        let mut data_offsets = Vec::new();
        let mut body = Vec::new();
        for s in &self.sections {
            let a = 1u64 << s.align;
            while !(offset + body.len() as u64).is_multiple_of(a) {
                body.push(0);
            }
            data_offsets.push(offset + body.len() as u64);
            body.extend_from_slice(&s.data);
        }
        while !body.len().is_multiple_of(8) {
            body.push(0);
        }
        let mut reloc_offsets = Vec::new();
        for s in &self.sections {
            reloc_offsets.push(offset + body.len() as u64);
            for r in &s.relocs {
                body.extend_from_slice(&r.address.to_le_bytes());
                let word = (r.symbolnum & 0xff_ffff)
                    | (u32::from(r.pcrel) << 24)
                    | (u32::from(r.length) << 25)
                    | (u32::from(r.external) << 27)
                    | (u32::from(r.kind) << 28);
                body.extend_from_slice(&word.to_le_bytes());
            }
        }
        let symoff = offset + body.len() as u64;
        let mut strings = vec![0u8];
        for sym in &self.symbols {
            let strx = strings.len() as u32;
            strings.extend_from_slice(sym.name.as_bytes());
            strings.push(0);
            let value = if sym.n_sect > 0 {
                addrs[usize::from(sym.n_sect) - 1] + sym.offset
            } else {
                sym.offset
            };
            body.extend_from_slice(&strx.to_le_bytes());
            body.push(sym.n_type);
            body.push(sym.n_sect);
            body.extend_from_slice(&sym.n_desc.to_le_bytes());
            body.extend_from_slice(&value.to_le_bytes());
        }
        let stroff = offset + body.len() as u64;
        body.extend_from_slice(&strings);
        offset = 0;
        let _ = offset;
        let mut out = Vec::new();
        let u32le = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
        let u64le = |v: &mut Vec<u8>, x: u64| v.extend_from_slice(&x.to_le_bytes());
        let name16 = |v: &mut Vec<u8>, n: &str| {
            let mut f = [0u8; 16];
            f[..n.len()].copy_from_slice(n.as_bytes());
            v.extend_from_slice(&f);
        };
        u32le(&mut out, 0xfeed_facf);
        u32le(&mut out, 0x0100_000c);
        u32le(&mut out, 0);
        u32le(&mut out, 1); // MH_OBJECT
        u32le(&mut out, 2);
        u32le(&mut out, sizeofcmds);
        u32le(&mut out, 0x2000); // MH_SUBSECTIONS_VIA_SYMBOLS
        u32le(&mut out, 0);
        u32le(&mut out, 0x19);
        u32le(&mut out, 72 + 80 * nsects);
        name16(&mut out, "");
        u64le(&mut out, 0);
        u64le(&mut out, vmsize);
        u64le(&mut out, 32 + u64::from(sizeofcmds));
        u64le(
            &mut out,
            self.sections.iter().map(|s| s.data.len() as u64).sum(),
        );
        u32le(&mut out, 7);
        u32le(&mut out, 7);
        u32le(&mut out, nsects);
        u32le(&mut out, 0);
        for (i, s) in self.sections.iter().enumerate() {
            name16(&mut out, s.name);
            name16(&mut out, s.segment);
            u64le(&mut out, addrs[i]);
            u64le(
                &mut out,
                if s.zerofill > 0 {
                    s.zerofill
                } else {
                    s.data.len() as u64
                },
            );
            u32le(
                &mut out,
                if s.zerofill > 0 {
                    0
                } else {
                    data_offsets[i] as u32
                },
            );
            u32le(&mut out, s.align);
            u32le(
                &mut out,
                if s.relocs.is_empty() {
                    0
                } else {
                    reloc_offsets[i] as u32
                },
            );
            u32le(&mut out, s.relocs.len() as u32);
            u32le(&mut out, s.flags);
            u32le(&mut out, 0);
            u32le(&mut out, 0);
            u32le(&mut out, 0);
        }
        u32le(&mut out, 0x2);
        u32le(&mut out, 24);
        u32le(&mut out, symoff as u32);
        u32le(&mut out, self.symbols.len() as u32);
        u32le(&mut out, stroff as u32);
        u32le(&mut out, strings.len() as u32);
        assert_eq!(out.len() as u64, 32 + u64::from(sizeofcmds));
        out.extend_from_slice(&body);
        out
    }
}

/// An `ar` archive (GNU format, with a `/` symbol table) of named members, each object's
/// external definitions listed in the symbol table.
pub fn archive(members: &[(&str, &Object)]) -> Vec<u8> {
    let mut entries: Vec<(String, usize)> = Vec::new();
    for (index, (_, object)) in members.iter().enumerate() {
        for symbol in &object.symbols {
            if symbol.n_type & 0x0f == 0x0f {
                entries.push((symbol.name.clone(), index));
            }
        }
    }
    let names: Vec<u8> = entries
        .iter()
        .flat_map(|(n, _)| n.bytes().chain([0]))
        .collect();
    let table_size = 4 + 4 * entries.len() + names.len();
    let header = |name: &str, size: usize| {
        let mut h = format!(
            "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            0, 0, 0, 644, size
        );
        h.truncate(60);
        h.into_bytes()
    };
    let mut offsets = Vec::new();
    let mut at = 8 + 60 + table_size + table_size % 2;
    for (_, object) in members {
        offsets.push(at);
        let size = object.bytes().len();
        at += 60 + size + size % 2;
    }
    let mut out = b"!<arch>\n".to_vec();
    out.extend(header("/", table_size));
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (_, member) in &entries {
        out.extend_from_slice(&(offsets[*member] as u32).to_be_bytes());
    }
    out.extend_from_slice(&names);
    if table_size % 2 == 1 {
        out.push(b'\n');
    }
    for (name, object) in members {
        let bytes = object.bytes();
        out.extend(header(&format!("{name}/"), bytes.len()));
        out.extend_from_slice(&bytes);
        if bytes.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}

/// `stp x29, x30, [sp, #-16]!; mov x29, sp`.
pub const PROLOGUE: [u32; 2] = [0xa9bf_7bfd, 0x9100_03fd];
/// `ldp x29, x30, [sp], #16; ret`.
pub const EPILOGUE: [u32; 2] = [0xa8c1_7bfd, 0xd65f_03c0];
/// `bl` with a zero offset.
pub const BL: u32 = 0x9400_0000;
/// `ret`.
pub const RET: u32 = 0xd65f_03c0;
/// `movz w0, #imm`.
pub fn mov_w0(imm: u16) -> u32 {
    0x5280_0000 | (u32::from(imm) << 5)
}
