//! The bit-for-bit corpus of the dense kernels (ADR 0120 rule 53): the references, the blocked
//! kernels and the baseline table of `una_core::dense::variants`, by `corpus::check`.
//!
//! The x86-64 variants are not called here: calling a `#[target_feature]` function needs
//! `unsafe`, which this crate forbids. `una-rt`'s `tests/dense_variants.rs` runs the same
//! corpus with every variant the processor runs; this test prints each one it skips.

mod corpus;

use una_core::dense::variants::{Kernels, variant_names};

#[test]
#[allow(clippy::print_stderr)]
fn every_kernel_reproduces_the_corpus_bit_for_bit() {
    corpus::check(&[Kernels::BASELINE]);
    // A skip is printed, never silent (ADR 0120 rule 53).
    for name in variant_names() {
        if *name == Kernels::BASELINE.name {
            eprintln!("ran: the {name} variant");
        } else {
            eprintln!(
                "skipped here: the {name} variant, which crates/una-rt/tests/dense_variants.rs runs \
                 where the processor has its features"
            );
        }
    }
}
