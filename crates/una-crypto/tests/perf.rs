//! Performance and scaling sanity (ADR 0133 rule 10): linear time in the message size, and a
//! floor on throughput so a quadratic or accidental slow path is caught. Optimized builds only;
//! the bounds are loose (an order of magnitude) because the machine is shared.

// Test code: unwrap, indexing and panics are how a test fails (clippy.toml allows them in `#[test]`
// functions only, not in their helpers).
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]

mod common;

use std::time::Instant;
use una_crypto::chacha20poly1305::seal_in_place;
use una_crypto::sha2::{Sha256, Sha512};
use una_crypto::x25519::x25519;

fn run(name: &str, m: &[u8]) {
    match name {
        "sha256" => {
            std::hint::black_box(Sha256::digest(m));
        }
        "sha512" => {
            std::hint::black_box(Sha512::digest(m));
        }
        _ => {
            let mut buf = m.to_vec();
            std::hint::black_box(seal_in_place(&[1u8; 32], &[2u8; 12], b"", &mut buf).ok());
        }
    }
}

fn seconds(mut f: impl FnMut()) -> f64 {
    f(); // warm up
    let best = (0..3)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed().as_secs_f64()
        })
        .fold(f64::MAX, f64::min);
    best.max(1e-9)
}

#[test]
fn time_is_linear_in_the_message_size() {
    if cfg!(debug_assertions) {
        return;
    }
    let small = vec![0u8; 4 << 20];
    let large = vec![0u8; 32 << 20];
    for name in ["sha256", "sha512", "chacha20poly1305"] {
        let t_small = seconds(|| run(name, &small));
        let t_large = seconds(|| run(name, &large));
        let ratio = t_large / t_small;
        // 8x the data: linear is 8; allow 4 to 16.
        assert!((4.0..16.0).contains(&ratio), "{name}: ratio {ratio:.2}");
        let mib_s = 32.0 / t_large;
        assert!(mib_s > 50.0, "{name}: {mib_s:.0} MiB/s");
    }
}

#[test]
fn one_scalar_multiplication_is_fast() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut p = [0u8; 32];
    p[0] = 9;
    let k = [0x55u8; 32];
    let t = seconds(|| {
        let mut x = p;
        for _ in 0..200 {
            x = x25519(&k, &x);
        }
        std::hint::black_box(x);
    }) / 200.0;
    assert!(t < 500e-6, "{:.1} us per X25519", t * 1e6);
}
