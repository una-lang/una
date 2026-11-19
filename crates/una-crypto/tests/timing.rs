//! A dudect-style statistical timing check (ADR 0133 rule 9): two input classes that differ only
//! in a secret are timed in interleaved random order, the slowest tail is cropped, and Welch's
//! t statistic must stay small. Every test uses one method (`measure_in_place`, #2153): the two
//! classes share one state, the input of the sample's class is written into it outside the
//! timed region by the same kind of work for both classes, and the first samples are discarded,
//! so the timed region holds the routine under test and nothing else. It is evidence for the release gate, not a proof: it runs on a
//! quiet machine with `cargo test --release --locked -p una-crypto --test timing -- --ignored
//! --test-threads=1 --nocapture` (parallel tests disturb each other's clocks), in the release gate
//! `timing-channels` of `scripts/release-gates.sh` and in the step of the same name of
//! `scripts/local-ci.sh` (#2119, `docs/release-gates.md`).

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

mod common;

use common::Rng;
use std::hint::black_box;
use std::time::Instant;
use una_crypto::aes_gcm::AesGcm;
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

/// Crops the slowest 5% of each class (interrupts, frequency changes) and returns Welch's t.
#[allow(clippy::print_stderr)] // the statistic is the evidence; --nocapture shows it
fn cropped_t(mut times: [Vec<f64>; 2]) -> f64 {
    for t in &mut times {
        t.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let keep = t.len() * 95 / 100;
        t.truncate(keep);
    }
    let t = welch(&times[0], &times[1]);
    eprintln!("timing t = {t:.2}");
    t
}

/// Samples discarded before the first one kept, by `measure_in_place`.
const WARM_UP: usize = 2_000;

/// Times `f(state)` for the two classes in a random interleaving and returns the cropped t
/// statistic. A sample is `batch` calls of `f` (more than one for a call of a few tens of
/// nanoseconds, which one reading of the clock cannot time), and the first `WARM_UP` samples
/// are discarded. The two classes share one `state`: `prepare(state, class)`, outside the timed
/// region, makes it an input of the class, so both classes read and write the same addresses
/// and nothing but the secret differs between them inside the timed region. Two inputs in two
/// allocations differ in alignment and cache placement too, which shows as a t of either sign;
/// an input prepared inside the timed region adds the time of its preparation, which differs by
/// class, to the statistic (#2153). `prepare` does the same kind of work for both classes.
fn measure_in_place<S>(
    batch: usize,
    state: &mut S,
    mut prepare: impl FnMut(&mut S, bool),
    mut f: impl FnMut(&mut S),
) -> f64 {
    let mut rng = Rng(0x71_3e);
    let mut times = [Vec::new(), Vec::new()];
    for sample in 0..WARM_UP + SAMPLES {
        let class = rng.next() & 1 == 1;
        prepare(state, class);
        let start = Instant::now();
        for _ in 0..batch {
            f(state);
        }
        let ns = start.elapsed().as_nanos() as f64;
        if sample >= WARM_UP {
            times[usize::from(class)].push(ns);
        }
    }
    cropped_t(times)
}

/// Writes `random` (class 1) or `fixed` (class 0) into `out` with no branch on the class: both
/// are read and every byte is written either way, so the preparation of a sample costs the same
/// for both classes and leaves the branch predictor in the same state before the timed region.
fn select<const N: usize>(out: &mut [u8; N], random: &[u8; N], fixed: &[u8; N], class: bool) {
    let mask = black_box(0u8.wrapping_sub(u8::from(class)));
    for ((o, r), f) in out.iter_mut().zip(random).zip(fixed) {
        *o = (r & mask) | (f & !mask);
    }
}

/// The input of a rejection test: the ciphertext and the tag presented with it.
struct Rejected {
    buf: Vec<u8>,
    tag: [u8; 16],
}

/// Makes `state` the ciphertext with a tag that differs from the real one in its first byte
/// (class 1) or in its last (class 0). Both bytes are written for both classes.
fn prepare_rejected(state: &mut Rejected, ciphertext: &[u8], real: &[u8; 16], class: bool) {
    state.buf.copy_from_slice(ciphertext);
    state.tag = *real;
    state.tag[0] ^= u8::from(class);
    state.tag[15] ^= u8::from(!class);
}

#[test]
#[ignore = "release evidence: needs a quiet machine and --release"]
fn comparison_time_does_not_depend_on_the_first_difference() {
    // One call takes a few tens of nanoseconds, so a sample is 64 calls. The other operand is
    // one buffer for both classes: before each sample, outside the timed region, it is made to
    // differ from `a` in its first byte (an early-exit comparison then returns at once) or in
    // its last (it reads everything). Both bytes are written each time, so the two classes
    // leave the same cache lines written.
    let a = vec![7u8; 4096];
    let mut other = a.clone();
    let t = measure_in_place(
        64,
        &mut other,
        |other, class| {
            other[0] = 7 ^ u8::from(class);
            other[4095] = 7 ^ u8::from(!class);
        },
        |other| {
            black_box(ct::eq(black_box(&a), black_box(&other[..])));
        },
    );
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
    // last, so an early-exit comparison would take less time for the first class. The buffer
    // and the tag are one state, restored and written outside the timed region.
    let mut state = Rejected {
        buf: ciphertext.clone(),
        tag: real,
    };
    let t = measure_in_place(
        1,
        &mut state,
        |state, class| prepare_rejected(state, &ciphertext, &real, class),
        |state| {
            let tag = black_box(&state.tag);
            black_box(open_in_place(&key, &nonce, b"", &mut state.buf, tag).is_err());
        },
    );
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
    // Class 1: the all-zero scalar, which clamps to a scalar with a single high bit set.
    // Class 0: the all-ones scalar. One buffer holds the scalar of the sample's class.
    let mut scalar = [0u8; 32];
    let t = measure_in_place(
        1,
        &mut scalar,
        |scalar, class| select(scalar, &[0u8; 32], &[0xffu8; 32], class),
        |scalar| {
            black_box(x25519::x25519(black_box(scalar), black_box(&point)));
        },
    );
    assert!(t.abs() < LIMIT, "x25519 t = {t}");
}

/// The input of a seal test: the key and the data sealed in place.
struct Sealed<const K: usize, const N: usize> {
    key: [u8; K],
    data: [u8; N],
}

#[test]
#[ignore = "release evidence: needs a quiet machine and --release"]
fn aes_gcm_time_does_not_depend_on_the_key_or_the_plaintext() {
    // Class 0: one fixed key and an all-zero plaintext. Class 1: keys and plaintexts drawn at
    // random beforehand. The key schedule, the hash subkey, the key stream and the tag are all
    // inside the timed region, and nothing else is: the key and the plaintext of the sample are
    // written into one state before it. Both classes read the next entry of the table there and
    // both write the whole state, with no branch on the class (`select`), so they leave the
    // same cache lines read and written (#2153:
    // with the entry copied inside the timed region, for the random class alone, the statistic
    // measured that copy).
    let mut rng = Rng(0xae5_6c3);
    let random_keys: Vec<([u8; 16], [u8; 256])> = (0..256)
        .map(|_| {
            let mut key = [0u8; 16];
            key.copy_from_slice(&rng.bytes(16));
            let mut data = [0u8; 256];
            data.copy_from_slice(&rng.bytes(256));
            (key, data)
        })
        .collect();
    let fixed_key = [0u8; 16];
    let nonce = [5u8; 12];
    let mut n = 0usize;
    let mut state = Sealed {
        key: [0u8; 16],
        data: [0u8; 256],
    };
    let t = measure_in_place(
        1,
        &mut state,
        |state, class| {
            n = (n + 1) % random_keys.len();
            let (key, data) = &random_keys[n];
            select(&mut state.key, key, &fixed_key, class);
            select(&mut state.data, data, &[0u8; 256], class);
        },
        |state| {
            let g = AesGcm::new_128(black_box(&state.key));
            black_box(
                g.seal_in_place(&nonce, b"aad", black_box(&mut state.data))
                    .unwrap(),
            );
        },
    );
    assert!(t.abs() < LIMIT, "aes-128-gcm seal t = {t}");

    // AES-256: the same plaintext for both classes, a fixed key against random keys.
    let fixed_key = [0u8; 32];
    let random_keys: Vec<[u8; 32]> = (0..256)
        .map(|_| rng.bytes(32).try_into().unwrap())
        .collect();
    let mut state = Sealed {
        key: [0u8; 32],
        data: [0u8; 64],
    };
    let t = measure_in_place(
        1,
        &mut state,
        |state, class| {
            n = (n + 1) % random_keys.len();
            select(&mut state.key, &random_keys[n], &fixed_key, class);
            state.data = [0xa5u8; 64];
        },
        |state| {
            let g = AesGcm::new_256(black_box(&state.key));
            black_box(
                g.seal_in_place(&nonce, b"", black_box(&mut state.data))
                    .unwrap(),
            );
        },
    );
    assert!(t.abs() < LIMIT, "aes-256-gcm seal t = {t}");
}

#[test]
#[ignore = "release evidence: needs a quiet machine and --release"]
fn aes_gcm_rejection_time_does_not_depend_on_the_tag_byte_that_differs() {
    let g = AesGcm::new_256(&[3u8; 32]);
    let nonce = [5u8; 12];
    let mut ciphertext = vec![9u8; 256];
    let real = g.seal_in_place(&nonce, b"", &mut ciphertext).unwrap();
    let mut state = Rejected {
        buf: ciphertext.clone(),
        tag: real,
    };
    let t = measure_in_place(
        1,
        &mut state,
        |state, class| prepare_rejected(state, &ciphertext, &real, class),
        |state| {
            let tag = black_box(&state.tag);
            black_box(g.open_in_place(&nonce, b"", &mut state.buf, tag).is_err());
        },
    );
    assert!(t.abs() < LIMIT, "aes-gcm open t = {t}");
}
