//! The stand-in solver of the test hooks (ADR 0124; `UNA_SOLVER_STAND_IN`) on a loaded machine
//! (#1573), shared by the tests that run one (`lsp.rs`, `verification.rs` include this file by
//! path).
//!
//! The parent waits 2 seconds for a solver child's handshake (ADR 0124 rule 4), counted from
//! just before the spawn, and ends a child that has written none by then with U5003, "the
//! solver child is not this `una`: no handshake within 2 seconds". A stand-in is a shell
//! script, which on a loaded machine can take longer than that to start and write its line:
//! the case it serves then never runs. A run that ends that way, and only that way, is the
//! environment's failure, so it is run again, up to [`ATTEMPTS`] runs in all; every other
//! outcome is judged by the case's own assertions, unchanged, and the last run is judged
//! whatever its outcome, so a case that misses its handshake in every run still fails, with
//! the reasons of the earlier runs in its message.

// Each test file uses its own subset of these helpers.
#![allow(dead_code)]

use std::process::Output;

use serde_json::Value;

/// The most runs of a case whose stand-in solver missed its handshake window.
pub const ATTEMPTS: usize = 5;

/// Whether the diagnostic `diagnostic` is the U5003 of a solver child that wrote no handshake
/// within the window of ADR 0124 rule 4, and nothing else.
pub fn late_handshake(diagnostic: &Value) -> bool {
    diagnostic["code"] == "U5003"
        && diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.ends_with("no handshake within 2 seconds"))
}

/// The message of the first diagnostic of `diagnostics`, an array, that is a late handshake.
pub fn late_in(diagnostics: &Value) -> Option<String> {
    diagnostics
        .as_array()?
        .iter()
        .find(|diagnostic| late_handshake(diagnostic))
        .map(|diagnostic| diagnostic["message"].as_str().unwrap_or("").to_owned())
}

/// Runs `run`, a `una --json check` with a stand-in solver, again while its document reports
/// a late handshake, up to [`ATTEMPTS`] runs in all, and returns the last run's output with
/// the reasons of the runs before it, for the case's messages.
pub fn check_with_stand_in(mut run: impl FnMut() -> Output) -> (Output, Vec<String>) {
    let mut late = Vec::new();
    loop {
        let output = run();
        if late.len() + 1 < ATTEMPTS {
            let document: Option<Value> = serde_json::from_slice(&output.stdout).ok();
            if let Some(reason) = document.and_then(|document| late_in(&document["diagnostics"])) {
                late.push(reason);
                continue;
            }
        }
        return (output, late);
    }
}
