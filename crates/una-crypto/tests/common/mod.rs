//! Shared helpers of the integration tests.
#![allow(dead_code, clippy::unwrap_used, clippy::indexing_slicing)]

use serde_json::Value;
use std::path::PathBuf;

pub fn hex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd hex length");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn array32(v: &[u8]) -> [u8; 32] {
    v.try_into().unwrap()
}

pub fn vector_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("vectors")
        .join(name)
}

/// A vendored Wycheproof file; its digest is pinned in `vectors/DIGESTS.sha256` and checked by
/// `tests/vectors_pinned.rs`.
pub fn wycheproof(file: &str) -> Value {
    let text = std::fs::read_to_string(vector_path(&format!("wycheproof/{file}"))).unwrap();
    serde_json::from_str(&text).unwrap()
}

pub fn tests_of(root: &Value) -> Vec<(&Value, &Value)> {
    let mut out = Vec::new();
    for group in root["testGroups"].as_array().unwrap() {
        for test in group["tests"].as_array().unwrap() {
            out.push((group, test));
        }
    }
    out
}

pub fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap()
}

/// A deterministic generator for the differential tests (xorshift64*; not for secrets).
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| (self.next() >> 24) as u8).collect()
    }
    /// Random bytes of a random length below `max`.
    pub fn vec(&mut self, max: usize) -> Vec<u8> {
        let n = self.below(max);
        self.bytes(n)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}
