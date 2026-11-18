//! The case lists of the tests of `una-cli` that run one test per case (#1652), so that a test
//! runner spreads the cases over all cores: the conformance cases, the runnable programs of the
//! native differential, the targets of the determinism test and the corpus of the Core IR
//! export test. The build script generates one macro per list from the repository's corpus;
//! each test file passes it a macro that writes one `#[test]` per entry and checks, in a test of
//! its own, that the generated list is the one on disk.

include!(concat!(env!("OUT_DIR"), "/cases.rs"));
