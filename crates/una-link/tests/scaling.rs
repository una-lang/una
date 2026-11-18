//! The cost of a link (ADR 0124, Costs; rule 33): O(N + S log S) for N bytes of input and S
//! sections, so linking a synthetic program grows linearly with its functions and with its
//! relocations per function. Each size is measured as the median of 9 links, and each doubling
//! must cost at most 2.3 times as much past the fixed cost (#341's re-measurement on a miss).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod common;
#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use common::*;

/// A program of `functions` functions, each calling the next and loading `loads` pointers from
/// `__DATA`, each pointer a rebase to a function.
fn synthetic(functions: usize, loads: usize) -> Vec<u8> {
    let mut code = Vec::new();
    let mut relocs = Vec::new();
    let mut symbols = vec![global("_main", 1, 0)];
    let mut pointers = Vec::new();
    let mut data_relocs = Vec::new();
    let mut starts = Vec::new();
    for index in 0..functions {
        starts.push(code.len() as u64 * 4);
        code.extend_from_slice(&PROLOGUE);
        for load in 0..loads {
            let slot = index * loads + load;
            let at = code.len() as u32 * 4;
            code.push(0x9000_0008); // adrp x8, d@PAGE
            code.push(0xf940_0109); // ldr x9, [x8, d@PAGEOFF]
            // Symbol indices: functions first (0..functions), then one per data slot.
            let data_symbol = (functions + slot) as u32;
            relocs.push(reloc(at, data_symbol, PAGE21));
            relocs.push(reloc(at + 4, data_symbol, PAGEOFF12));
            data_relocs.push(reloc(slot as u32 * 8, index as u32, UNSIGNED));
            pointers.extend_from_slice(&[0; 8]);
        }
        if index + 1 < functions {
            relocs.push(reloc(code.len() as u32 * 4, (index + 1) as u32, BRANCH26));
            code.push(BL);
        }
        code.push(mov_w0(0));
        code.extend_from_slice(&EPILOGUE);
    }
    for (index, start) in starts.iter().enumerate().skip(1) {
        symbols.push(local(&format!("_f{index}"), 1, *start));
    }
    for slot in 0..functions * loads {
        symbols.push(local(&format!("_d{slot}"), 2, slot as u64 * 8));
    }
    let mut text_section = text(&code);
    text_section.relocs = relocs;
    let mut data_section = data("__DATA", "__data", 0, &pointers);
    data_section.relocs = data_relocs;
    Object {
        sections: vec![text_section, data_section],
        symbols,
    }
    .bytes()
}

fn median_of_nine(program: &[u8], runtime: &[u8]) -> f64 {
    let mut times: Vec<f64> = (0..9)
        .map(|_| {
            timing::timed(|| {
                una_link::link(
                    program,
                    runtime,
                    &una_link::Options {
                        identifier: "s",
                        debuggable: false,
                    },
                )
                .unwrap()
            })
            .1
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[4]
}

#[test]
fn a_link_grows_linearly_with_functions_and_with_relocations() {
    let runtime = archive(&[(
        "unused.o",
        &Object {
            sections: vec![text(&[RET])],
            symbols: vec![global("_unused", 1, 0)],
        },
    )]);
    let fixed = timing::best_of_three(|| median_of_nine(&synthetic(1, 1), &runtime));
    let programs: std::collections::BTreeMap<usize, Vec<u8>> = [1_000, 2_000, 4_000, 8_000]
        .iter()
        .map(|&n| (n, synthetic(n, 1)))
        .collect();
    timing::assert_growth(
        "functions",
        &[1_000, 2_000, 4_000, 8_000],
        fixed,
        1e-4,
        2.3,
        |n| median_of_nine(&programs[&n], &runtime),
    );
    let loads: std::collections::BTreeMap<usize, Vec<u8>> = [2, 4, 8, 16]
        .iter()
        .map(|&k| (k, synthetic(1_000, k)))
        .collect();
    timing::assert_growth(
        "relocations per function",
        &[2, 4, 8, 16],
        fixed,
        1e-4,
        2.3,
        |k| median_of_nine(&loads[&k], &runtime),
    );
}
