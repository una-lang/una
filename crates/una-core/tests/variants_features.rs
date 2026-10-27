//! Exactly the features detected (ADR 0120 rule 56): each x86-64 variant of
//! `una_core::dense::variants` enables, by `#[target_feature]`, exactly the features its
//! detection checks. Rust's target features imply others (`avx` implies `sse4.2`; `avx512f`
//! implies `avx2`, `fma` and `f16c`), so this test asks the toolchain itself for the closure of
//! each variant's list under its implications, as the features that `rustc --print cfg` gains
//! over the bare target, and asserts that the closure is the list: no feature the variant can
//! use goes undetected.
//!
//! It asks about `x86_64-unknown-linux-gnu` whatever the host is, so it runs on every host;
//! `--print cfg` needs no standard library for the target. Without a toolchain `rustc` it
//! fails with the reason, never skips.

// Tests may unwrap and panic freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::process::Command;

use una_core::dense::variants::{X86_64_V3_FEATURES, X86_64_V4_FEATURES};

/// The target whose implications the variants are compiled under.
const TARGET: &str = "x86_64-unknown-linux-gnu";

/// The toolchain's `rustc`: `RUSTC` when set, else `rustc` on `PATH`, which rustup resolves to
/// the toolchain of `rust-toolchain.toml` from this crate's directory, where tests run.
fn rustc() -> OsString {
    // A test, not the library: it may read the environment (compiler/conventions.md §1).
    #[allow(clippy::disallowed_methods)]
    let configured = std::env::var_os("RUSTC");
    configured.unwrap_or_else(|| OsString::from("rustc"))
}

/// The `target_feature` values of `rustc --print cfg` for [`TARGET`], with `enabled` added by
/// `-C target-feature` when it is not empty.
fn cfg_features(enabled: &[&str]) -> BTreeSet<String> {
    let rustc = rustc();
    let mut command = Command::new(&rustc);
    command.args(["--print", "cfg", "--target", TARGET]);
    if !enabled.is_empty() {
        let list: Vec<String> = enabled
            .iter()
            .map(|feature| format!("+{feature}"))
            .collect();
        command
            .arg("-C")
            .arg(format!("target-feature={}", list.join(",")));
    }
    let output = command.output().unwrap_or_else(|error| {
        panic!(
            "ADR 0120 rule 56 asks the toolchain for the implications of target features, and \
             {rustc:?} did not run ({error}): put the toolchain's rustc on PATH or set RUSTC"
        )
    });
    assert!(
        output.status.success(),
        "{rustc:?} --print cfg --target {TARGET} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("target_feature=\""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .map(str::to_owned)
        .collect()
}

/// Asserts that the features `list` adds to the bare target are exactly `list`.
fn closure_is_the_list(name: &str, list: &[&str]) {
    let bare = cfg_features(&[]);
    assert!(
        bare.contains("sse2"),
        "the bare {TARGET} has its baseline features: {bare:?}"
    );
    let enabled = cfg_features(list);
    let added: BTreeSet<String> = enabled.difference(&bare).cloned().collect();
    let detected: BTreeSet<String> = list.iter().map(|feature| (*feature).to_owned()).collect();
    assert_eq!(
        added, detected,
        "{name}: the closure of its target features under the toolchain's implications (left) \
         must be exactly the features its detection checks (right), ADR 0120 rule 56"
    );
}

#[test]
fn the_x86_64_v3_variant_enables_exactly_the_features_it_detects() {
    closure_is_the_list("x86_64_v3", X86_64_V3_FEATURES);
}

#[test]
fn the_x86_64_v4_variant_enables_exactly_the_features_it_detects() {
    closure_is_the_list("x86_64_v4", X86_64_V4_FEATURES);
}

#[test]
fn the_v4_variant_detects_every_feature_of_the_v3_variant() {
    for feature in X86_64_V3_FEATURES {
        assert!(X86_64_V4_FEATURES.contains(feature), "{feature}");
    }
}
