//! `cargo run -p una-core --example contraction-canary`: the contraction-sensitive case of the
//! float corpus (ADR 0114 rule 16.4, the `fma` line of `tests/conformance/runtime/float_run`),
//! the subject of the x86-64 contraction canary of `scripts/fma-canary.sh` (#1228; ADR 0114
//! Appendix C rule 7, as ADR 0120 Appendix A amends it in its amendment of ADR 0106, item 3).
//!
//! It computes, with inputs the compiler cannot see, `a * b + c` with `a = 1 + 2^-52`,
//! `b = 1 - 2^-52` and `c = -1`, which is two roundings, `0.0`, by ADR 0114 rule 6.2 and `-2^-104`
//! when a compiler contracts it into one fused instruction; and the same triple through
//! [`una_core::dense::fma::emulated`], the scalar `fma` of the dense kernels on x86-64 without
//! FMA3, whose error-free products also change when a compiler contracts them. It prints the
//! bits of both results, one line each: `case 0x...` and `emulated 0x...`.
//!
//! Built with the shipped flags it holds no FMA instruction, and `case` is `0x0000000000000000`.
//! Built with contraction forced on (`-C target-feature=+fma -C llvm-args=-fp-contract=fast`,
//! Rust's equivalent of `-mfma -ffp-contract=fast`), the FMA gate of `scripts/check-fma.sh` must
//! fail on [`contraction_case`], and where the processor has FMA, `case` is the fused
//! `0xb970000000000000`: the corpus fails on its contraction-sensitive case.
#![allow(clippy::print_stdout)]

use std::hint::black_box;

/// `a * b + c`, two roundings in every shipped build (ADR 0114 rule 6.2): the expression a
/// contracting compiler fuses. Never inlined, so the FMA gate can name it. O(1).
#[inline(never)]
pub fn contraction_case(a: f64, b: f64, c: f64) -> f64 {
    a * b + c
}

fn main() {
    // 1 + 2^-52, 1 - 2^-52 and -1, through `black_box` so that nothing is folded at build time.
    let a = black_box(f64::from_bits(0x3FF0_0000_0000_0001));
    let b = black_box(f64::from_bits(0x3FEF_FFFF_FFFF_FFFE));
    let c = black_box(-1.0f64);
    println!("case {:#018x}", contraction_case(a, b, c).to_bits());
    println!(
        "emulated {:#018x}",
        una_core::dense::fma::emulated(a, b, c).to_bits()
    );
}
