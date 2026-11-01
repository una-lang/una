//! A dudect-style statistical timing check (ADR 0133 rule 9): two input classes that differ only
//! in a secret are timed in interleaved random order, the slowest tail is cropped, and Welch's
//! t statistic must stay small. It is evidence for the release gate, not a proof: it runs on a
//! quiet machine with `cargo test --release -p una-crypto --test timing -- --ignored --test-threads=1` (parallel tests disturb each other's clocks).

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

mod common;

use common::Rng;
use std::hint::black_box;
use std::time::Instant;
use una_crypto::chacha20poly1305::{open_in_place, seal_in_place};
use una_crypto::{ct, x25519};

const SAMPLES: usize = 20_000;
/// |t| above this is a leak by dudect's convention (4.5 is already suspicious; 10 is sure).
const LIMIT: f64 = 10.0;

fn welch(a: &[f64], b: &[f64]) -> f64 {
    let stat = |v: &[f64]| {
        let n = v.len() as f64;
        let mean = v.iter().sum::<f64>() / n;
        let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0);
        (mean, var / n)
    };
    let (ma, va) = stat(a);
    let (mb, vb) = stat(b);
    (ma - mb) / (va + vb).sqrt()
}

/// Times `f(class)` for the two classes in random interleaving; returns the cropped t statistic.
fn measure(mut f: impl FnMut(bool)) -> f64 {
    let mut rng = Rng(0x71_3e);
    let mut times = [Vec::new(), Vec::new()];
    for _ in 0..SAMPLES {
        let class = rng.next() & 1 == 1;
        let start = Instant::now();
        f(class);
        let ns = start.elapsed().as_nanos() as f64;
        times[usize::from(class)].push(ns);
    }
    // Crop the slowest 5% of each class (interrupts, frequency changes).
    for t in &mut times {
        t.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let keep = t.len() * 95 / 100;
        t.truncate(keep);
    }
    welch(&times[0], &times[1])
}

#[test]
#[ignore = "release evidence: needs a quiet machine and --release"]
fn comparison_time_does_not_depend_on_the_first_difference() {
    let a = vec![7u8; 4096];
    let mut early = a.clone();
    early[0] ^= 1;
    let mut late = a.clone();
    late[4095] ^= 1;
    let t = measure(|class| {
        let other = if class { &early } else { &late };
        black_box(ct::eq(black_box(&a), black_box(other)));
    });
    assert!(t.abs() < LIMIT, "ct::eq t = {t}");
}

#[test]
#[ignore = "release evidence: needs a quiet machine and --release"]
fn aead_rejection_time_does_not_depend_on_the_tag_byte_that_differs() {
    let key = [3u8; 32];
    let nonce = [5u8; 12];
    let mut ciphertext = vec![9u8; 256];
    let real = seal_in_place(&key, &nonce, b"", &mut ciphertext).unwrap();
    // Both classes are rejected; they differ from the real tag in the first byte or in the
    // last, so an early-exit comparison would take less time for the first class.
    let mut flip_first = real;
    flip_first[0] ^= 1;
    let mut flip_last = real;
    flip_last[15] ^= 1;
    let t = measure(|class| {
        let mut buf = ciphertext.clone();
        let tag = if class { &flip_first } else { &flip_last };
        black_box(open_in_place(&key, &nonce, b"", &mut buf, black_box(tag)).is_err());
    });
    assert!(t.abs() < LIMIT, "aead open t = {t}");
}

#[test]
#[ignore = "release evidence: needs a quiet machine and --release"]
fn x25519_time_does_not_depend_on_the_scalar() {
    let point = {
        let mut p = [0u8; 32];
        p[0] = 9;
        p
    };
    let low = [0u8; 32]; // clamps to a scalar with a single high bit set
    let high = [0xffu8; 32];
    let t = measure(|class| {
        let k = if class { &low } else { &high };
        black_box(x25519::x25519(black_box(k), black_box(&point)));
    });
    assert!(t.abs() < LIMIT, "x25519 t = {t}");
}
