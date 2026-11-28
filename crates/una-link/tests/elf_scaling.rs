//! The cost of the ELF reader and relocation engine (ADR 0124, Costs; the project's complexity
//! rule): reading an object is O(size), and planning and applying its relocations is
//! O(R log R) for R relocations per section (a sort by offset, then O(1) per relocation), with
//! O(S) work per section for S sections, so `una_link::check_elf` on a synthetic arm64 object
//! grows linearly with its functions, with its relocations per function, and with its sections.
//! Each size is the median of 9 runs; each doubling must cost at most 2.3 times as much past the
//! fixed cost (#341's re-measurement on a miss).
//!
//! Names (review M1 and M2 of #1435): N symbols whose `st_name` all point at one string of S
//! bytes. A name is found in O(log) through an index of the string table's NULs, never by a
//! scan to its NUL, and no name is copied but into a reported message, so the reader, the
//! engine's check and the scan of 4a take time linear in the input; the names' total length,
//! counted with their overlaps, is bounded by 4 times the input (U8001 past it), so an input
//! whose names overlap N x S bytes is refused, in linear time. The same holds for symbols in a
//! section the executable drops, whose problem is formatted only when a relocation reaches one
//! (before, each held a copy of its name: memory N x S). The names' cases are judged at most 3
//! times per doubling and at most 1.5 times linear over their whole range (#2325): their sizes
//! are sub-millisecond, and one doubling can cross a cache size of the processor.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use std::collections::BTreeMap;

/// `R_AARCH64_CALL26`, `ADR_PREL_PG_HI21`, `ADD_ABS_LO12_NC` and `ABS64`.
const CALL26: u32 = 283;
const PAGE: u32 = 275;
const LO12: u32 = 277;
const ABS64: u32 = 257;

/// A section: name, flags, bytes, relocations (offset, symbol, type).
type Section = (String, u64, Vec<u8>, Vec<(u64, u32, u32)>);

/// An arm64 object of `functions` functions, each in its own section when `split`, each calling
/// the next and taking `loads` data addresses through `adrp`/`add`; `.data` holds one pointer
/// per function to it.
fn synthetic(functions: usize, loads: usize, split: bool) -> Vec<u8> {
    // Sections: the code sections, `.data`, then one `.rela` per section with relocations,
    // `.symtab`, `.strtab`, `.shstrtab`. Symbols: one per function, then `data`.
    let code_sections = if split { functions } else { 1 };
    let mut sections: Vec<Section> = Vec::new();
    let mut code = Vec::new();
    let mut relocs = Vec::new();
    let mut starts = Vec::new();
    let data_symbol = u32::try_from(functions + 1).unwrap();
    for index in 0..functions {
        if split && index > 0 {
            sections.push((format!(".text.f{}", index - 1), 0x6, code, relocs));
            code = Vec::new();
            relocs = Vec::new();
        }
        starts.push(code.len() as u64);
        for _ in 0..loads {
            relocs.push((code.len() as u64, data_symbol, PAGE));
            code.extend_from_slice(&0x9000_0008u32.to_le_bytes());
            relocs.push((code.len() as u64, data_symbol, LO12));
            code.extend_from_slice(&0x9100_0108u32.to_le_bytes());
        }
        if index + 1 < functions {
            relocs.push((code.len() as u64, u32::try_from(index + 2).unwrap(), CALL26));
            code.extend_from_slice(&0x9400_0000u32.to_le_bytes());
        }
        code.extend_from_slice(&0xd65f_03c0u32.to_le_bytes());
    }
    sections.push((format!(".text.f{}", functions - 1), 0x6, code, relocs));
    let data_relocs = (0..functions)
        .map(|i| (i as u64 * 8, u32::try_from(i + 1).unwrap(), ABS64))
        .collect();
    sections.push((".data".to_owned(), 0x3, vec![0; functions * 8], data_relocs));
    // Symbol i+1 is function i, in section 1+i when split (else 1, at its start).
    let mut symbols: Vec<(String, u16, u64)> = (0..functions)
        .map(|i| {
            let (section, value) = if split { (i + 1, 0) } else { (1, starts[i]) };
            (format!("f{i}"), u16::try_from(section).unwrap(), value)
        })
        .collect();
    symbols.push((
        "data".to_owned(),
        u16::try_from(code_sections + 1).unwrap(),
        0,
    ));
    write(&sections, &symbols)
}

/// A 64-bit arm64 relocatable object of `sections` (name, flags, data, relocations) and global
/// `symbols` (name, section, value).
fn write(sections: &[Section], symbols: &[(String, u16, u64)]) -> Vec<u8> {
    let mut shstrtab = vec![0u8];
    let mut name = |text: &str| {
        let at = shstrtab.len() as u64;
        shstrtab.extend_from_slice(text.as_bytes());
        shstrtab.push(0);
        at
    };
    let mut strtab = vec![0u8];
    let mut symtab = vec![0u8; 24];
    for (text, section, value) in symbols {
        let at = u32::try_from(strtab.len()).unwrap();
        strtab.extend_from_slice(text.as_bytes());
        strtab.push(0);
        symtab.extend_from_slice(&at.to_le_bytes());
        symtab.extend_from_slice(&[0x12, 0]);
        symtab.extend_from_slice(&section.to_le_bytes());
        symtab.extend_from_slice(&value.to_le_bytes());
        symtab.extend_from_slice(&0u64.to_le_bytes());
    }
    let mut body = Vec::new();
    let mut headers: Vec<[u64; 8]> = vec![[0; 8]];
    let place = |body: &mut Vec<u8>, bytes: &[u8]| {
        while !body.len().is_multiple_of(8) {
            body.push(0);
        }
        let at = 64 + body.len() as u64;
        body.extend_from_slice(bytes);
        at
    };
    let with_relocs = sections.iter().filter(|s| !s.3.is_empty()).count() as u64;
    let symtab_index = 1 + sections.len() as u64 + with_relocs;
    for (text, flags, data, _) in sections {
        let at = place(&mut body, data);
        headers.push([
            name(text) | (1 << 32),
            *flags,
            0,
            at,
            data.len() as u64,
            0,
            8,
            0,
        ]);
    }
    for (index, (text, _, _, relocs)) in sections.iter().enumerate() {
        if relocs.is_empty() {
            continue;
        }
        let mut rela = Vec::new();
        for &(offset, symbol, kind) in relocs {
            rela.extend_from_slice(&offset.to_le_bytes());
            rela.extend_from_slice(&((u64::from(symbol) << 32) | u64::from(kind)).to_le_bytes());
            rela.extend_from_slice(&0i64.to_le_bytes());
        }
        let at = place(&mut body, &rela);
        headers.push([
            name(&format!(".rela{text}")) | (4 << 32),
            0x40,
            0,
            at,
            rela.len() as u64,
            symtab_index | ((index as u64 + 1) << 32),
            8,
            24,
        ]);
    }
    let at = place(&mut body, &symtab);
    headers.push([
        name(".symtab") | (2 << 32),
        0,
        0,
        at,
        symtab.len() as u64,
        (symtab_index + 1) | (1 << 32),
        8,
        24,
    ]);
    let at = place(&mut body, &strtab);
    headers.push([
        name(".strtab") | (3 << 32),
        0,
        0,
        at,
        strtab.len() as u64,
        0,
        1,
        0,
    ]);
    let shstrtab_name = name(".shstrtab");
    let names = shstrtab.clone();
    let at = place(&mut body, &names);
    headers.push([
        shstrtab_name | (3 << 32),
        0,
        0,
        at,
        names.len() as u64,
        0,
        1,
        0,
    ]);
    while !body.len().is_multiple_of(8) {
        body.push(0);
    }
    let count = u16::try_from(headers.len()).unwrap();
    let mut out = b"\x7fELF\x02\x01\x01\x00".to_vec();
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&183u16.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&(64 + body.len() as u64).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for field in [64u16, 0, 0, 64, count, count - 1] {
        out.extend_from_slice(&field.to_le_bytes());
    }
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
        for field in [flags, addr, offset, size] {
            out.extend_from_slice(&field.to_le_bytes());
        }
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

/// An x86-64 object of `.text` (16 bytes), `.comment` (not loaded, 16 bytes), and `symbols`
/// local symbols whose names are all the string at offset 1 of a string table of `name`
/// bytes of `A`, defined in `.comment` when `dropped`, else in `.text`.
fn shared_names(symbols: usize, name: usize, dropped: bool) -> Vec<u8> {
    let mut strtab = vec![0u8];
    strtab.extend(std::iter::repeat_n(b'A', name));
    strtab.push(0);
    let mut symtab = vec![0u8; 24];
    for _ in 0..symbols {
        symtab.extend_from_slice(&1u32.to_le_bytes());
        symtab.extend_from_slice(&[0x01, 0]); // local object
        symtab.extend_from_slice(&(if dropped { 2u16 } else { 1u16 }).to_le_bytes());
        symtab.extend_from_slice(&[0; 16]);
    }
    let shstrtab = b"\0.text\0.comment\0.symtab\0.strtab\0.shstrtab\0".to_vec();
    let mut body = Vec::new();
    let mut place = |bytes: &[u8]| {
        while !body.len().is_multiple_of(8) {
            body.push(0);
        }
        let at = 64 + body.len() as u64;
        body.extend_from_slice(bytes);
        at
    };
    let text = place(&[0xc3; 16]);
    let comment = place(&[0; 16]);
    let sym = place(&symtab);
    let str_at = place(&strtab);
    let shstr = place(&shstrtab);
    while !body.len().is_multiple_of(8) {
        body.push(0);
    }
    let locals = (symbols + 1) as u64;
    let headers: [[u64; 8]; 6] = [
        [0; 8],
        [1 | (1 << 32), 0x6, 0, text, 16, 0, 16, 0],
        [7 | (1 << 32), 0, 0, comment, 16, 0, 1, 0],
        [
            16 | (2 << 32),
            0,
            0,
            sym,
            symtab.len() as u64,
            4 | (locals << 32),
            8,
            24,
        ],
        [24 | (3 << 32), 0, 0, str_at, strtab.len() as u64, 0, 1, 0],
        [32 | (3 << 32), 0, 0, shstr, shstrtab.len() as u64, 0, 1, 0],
    ];
    let mut out = b"\x7fELF\x02\x01\x01\x00".to_vec();
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&62u16.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&(64 + body.len() as u64).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for field in [64u16, 0, 0, 64, 6, 5] {
        out.extend_from_slice(&field.to_le_bytes());
    }
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
        for field in [flags, addr, offset, size] {
            out.extend_from_slice(&field.to_le_bytes());
        }
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

/// The most one doubling of a names case may cost (#2325): above 2.3, so that one step across a
/// cache size of the processor (a sub-millisecond size whose working set leaves a cache; 2.4x to
/// 2.5x on an x86-64 Xeon 6975P-C) passes, and below the 4 of a cost quadratic in N.
const NAMES_STEP: f64 = 3.0;

/// The most the whole range of a names case may cost, as a multiple of its growth in size
/// (#2325): 1.5 times linear, under the 12.2x that three doublings at 2.3 allowed before, so a
/// cost that grows faster than linear over the range still fails, whichever step it hides in.
const NAMES_RANGE: f64 = 1.5;

/// Asserts that `measure` grows linearly over `sizes` (#2325): each doubling within
/// [`NAMES_STEP`], and the last size within [`NAMES_RANGE`] times linear of the first, past
/// `fixed`, both judged by [`timing::assert_growth`] with its retries.
fn assert_names_linear(
    what: &str,
    sizes: &[usize],
    fixed: f64,
    mut measure: impl FnMut(usize) -> f64,
) {
    timing::assert_growth(what, sizes, fixed, 1e-4, NAMES_STEP, &mut measure);
    let ends = [sizes[0], sizes[sizes.len() - 1]];
    let range = ends[1] as f64 / ends[0] as f64;
    timing::assert_growth(what, &ends, fixed, 1e-4, range * NAMES_RANGE, &mut measure);
}

/// The median of 9 runs of `run`, in seconds.
fn median_of_nine_of(run: impl Fn()) -> f64 {
    let mut times: Vec<f64> = (0..9).map(|_| timing::timed(&run).1).collect();
    times.sort_by(f64::total_cmp);
    times[4]
}

#[test]
fn names_that_share_one_long_string_cost_linear_time_and_memory() {
    // Within the budget: 64-byte names shared by many symbols pass, in time linear in N.
    let ok = shared_names(1_000, 64, false);
    assert_eq!(una_link::check_elf("names.o", &ok).unwrap().objects, 1);
    assert!(una_link::scan_elf("names.o", &ok).is_ok());
    assert_eq!(
        una_link::check_elf("dropped.o", &shared_names(1_000, 64, true))
            .unwrap()
            .objects,
        1
    );
    // Past it: N symbols naming one string of S bytes, N and S doubling with the input, as the
    // review's reproducer (1.28 MB took 6.3 s); refused, each in time linear in the input.
    let sizes = [2_500, 5_000, 10_000, 20_000];
    for dropped in [false, true] {
        let inputs: BTreeMap<usize, Vec<u8>> = sizes
            .iter()
            .map(|&n| (n, shared_names(n, n * 40, dropped)))
            .collect();
        for bytes in inputs.values() {
            let problem = una_link::check_elf("shared.o", bytes).unwrap_err();
            assert!(problem.message.contains("4 times its size"), "{problem}");
            assert!(una_link::scan_elf("shared.o", bytes).is_err());
        }
        let tiny = shared_names(1, 40, dropped);
        let fixed = timing::best_of_three(|| {
            median_of_nine_of(|| {
                let _ = una_link::check_elf("shared.o", &tiny);
                let _ = una_link::scan_elf("shared.o", &tiny);
            })
        });
        let what = if dropped {
            "dropped symbols sharing a long name"
        } else {
            "symbols sharing a long name"
        };
        assert_names_linear(what, &sizes, fixed, |n| {
            median_of_nine_of(|| {
                let _ = una_link::check_elf("shared.o", &inputs[&n]);
                let _ = una_link::scan_elf("shared.o", &inputs[&n]);
            })
        });
    }
    // Within the budget, many symbols in a dropped section: linear in N.
    let inputs: BTreeMap<usize, Vec<u8>> = sizes
        .iter()
        .map(|&n| (n, shared_names(n * 4, 16, true)))
        .collect();
    assert_names_linear("dropped symbols", &sizes, 0.0, |n| {
        median_of_nine_of(|| {
            una_link::check_elf("dropped.o", &inputs[&n]).unwrap();
        })
    });
}

/// The median of 9 checks of `object`, in seconds.
fn median_of_nine(object: &[u8]) -> f64 {
    let mut times: Vec<f64> = (0..9)
        .map(|_| timing::timed(|| una_link::check_elf("synthetic.o", object).unwrap()).1)
        .collect();
    times.sort_by(f64::total_cmp);
    times[4]
}

#[test]
fn the_engine_grows_linearly_with_functions_relocations_and_sections() {
    let check = una_link::check_elf("synthetic.o", &synthetic(100, 2, true)).unwrap();
    assert_eq!(check.relocations["R_AARCH64_CALL26 direct"], 99);
    assert_eq!(check.relocations["R_AARCH64_ABS64 relative"], 100);
    let fixed = timing::best_of_three(|| median_of_nine(&synthetic(1, 1, false)));
    let sizes = [2_000, 4_000, 8_000, 16_000];
    for (what, split) in [("functions", false), ("sections", true)] {
        let objects: BTreeMap<usize, Vec<u8>> =
            sizes.iter().map(|&n| (n, synthetic(n, 1, split))).collect();
        timing::assert_growth(what, &sizes, fixed, 1e-4, 2.3, |n| {
            median_of_nine(&objects[&n])
        });
    }
    let loads: BTreeMap<usize, Vec<u8>> = [4, 8, 16, 32]
        .iter()
        .map(|&k| (k, synthetic(2_000, k, false)))
        .collect();
    timing::assert_growth(
        "relocations per function",
        &[4, 8, 16, 32],
        fixed,
        1e-4,
        2.3,
        |k| median_of_nine(&loads[&k]),
    );
}

#[test]
fn the_names_bound_passes_one_cache_step_and_fails_a_quadratic() {
    // #2325: the measures of the x86-64 runner pass (one doubling at 2.47x), a cost quadratic
    // in N fails at every doubling, and a cost linear but for one 3.5x step, or one that
    // grows 2.29x at each doubling (12x over the range, past 1.5 times linear), fails too.
    let sizes = [2_500, 5_000, 10_000, 20_000];
    let at = |times: [f64; 4]| move |n: usize| times[sizes.iter().position(|&s| s == n).unwrap()];
    assert_names_linear(
        "runner",
        &sizes,
        0.0,
        at([8.5248e-5, 0.000171343, 0.000423039, 0.000893494]),
    );
    for (case, times) in [
        ("quadratic", [1e-3, 4e-3, 16e-3, 64e-3]),
        ("one 3.5x step", [1e-3, 2e-3, 7e-3, 14e-3]),
        (
            "2.29x at each doubling",
            [1e-3, 2.29e-3, 5.2441e-3, 12.009e-3],
        ),
    ] {
        let failed = std::panic::catch_unwind(|| assert_names_linear(case, &sizes, 0.0, at(times)));
        assert!(failed.is_err(), "{case} passed");
    }
}
