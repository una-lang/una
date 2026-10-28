//! Every kernel variant this processor runs gives the corpus's bits (ADR 0120 rule 53): the
//! corpus of `una-core`'s `tests/dense_corpus.txt`, checked with every table of
//! `una_rt::ffi::runnable`, the baseline and each x86-64 variant whose exact features the
//! processor has (rule 56). A variant the processor cannot run is printed as skipped, never
//! passed over silently; the variant the runtime dispatches to is printed too.

#[path = "../../una-core/tests/corpus/mod.rs"]
mod corpus;

use una_core::dense::variants::variant_names;

#[test]
#[allow(clippy::print_stderr)]
fn every_variant_this_processor_runs_reproduces_the_corpus_bit_for_bit() {
    let tables = una_rt::ffi::runnable();
    corpus::check(&tables);
    for name in variant_names() {
        if tables.iter().any(|table| table.name == *name) {
            eprintln!("ran: the {name} variant");
        } else {
            eprintln!("skipped: the {name} variant, whose features this processor lacks");
        }
    }
    eprintln!("dispatched: the {} variant", una_rt::ffi::dispatched().name);
    assert!(
        tables
            .iter()
            .any(|table| table.name == una_rt::ffi::dispatched().name),
        "the dispatched variant is one this processor runs"
    );
}
