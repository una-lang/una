//! The bit-for-bit corpus of the dense kernels (ADR 0120 rule 53, issue #988): every case of
//! `dense_corpus.txt` is reproduced by the reference chains of `una_core::dense` and by the
//! blocked kernels of `una_core::dense::blocked`, and every `fma` and `ldexp` case by the
//! kernels' `fma` and by the reference routines of `una_core::float`.
//!
//! `scripts/dense-corpus.py` writes the corpus from an implementation of the chains of its own,
//! with exact rational arithmetic for `fma` and `ldexp`, and documents its format. The inputs
//! of most cases are named generators, which this test implements too; each case carries the
//! digest of its inputs, so a generator that drifts fails as such, before any kernel runs.

// Tests may unwrap and panic freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::collections::BTreeMap;

use una_core::dense::{self, Triangle, blocked, fma};
use una_core::float;

const CORPUS: &str = include_str!("dense_corpus.txt");

/// xorshift64 with shifts 13, 7, 17, from the corpus's state of a seed.
struct Draws(u64);

impl Draws {
    fn new(seed: u64) -> Draws {
        let state = seed
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(0x2545_F491_4F6C_DD1D);
        Draws(if state == 0 { 1 } else { state })
    }

    fn next(&mut self) -> u64 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        self.0 = s;
        s
    }
}

fn uniform(r: u64) -> f64 {
    // `f64::EPSILON` is exactly 2^-52 (`powi` may call the platform C library).
    (r >> 11) as f64 * f64::EPSILON - 1.0
}

/// The special values of the generator `s`, in its order.
fn specials() -> [f64; 14] {
    [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.0,
        -0.0,
        5.0e-324,
        -f64::MIN_POSITIVE,
        f64::MAX,
        -f64::MAX,
        1.0e300,
        -1.0e-300,
        3.0 * 5.0e-324,
        1.0,
        -1.0,
    ]
}

fn value(kind: &str, draws: &mut Draws) -> f64 {
    let r = draws.next();
    match kind {
        "u" => uniform(r),
        "w" => {
            let exponent = (r >> 3) % 200;
            // 1023 + (exponent - 100), the biased exponent.
            f64::from_bits(((r >> 63) << 63) | ((923 + exponent) << 52) | (r & ((1 << 52) - 1)))
        }
        "s" => {
            let pick = usize::try_from(r % 16).unwrap();
            let table = specials();
            if pick < table.len() {
                table[pick]
            } else if pick == table.len() {
                float::canonical(f64::from_bits(draws.next()))
            } else {
                uniform(draws.next())
            }
        }
        _ => panic!("no generator `{kind}`"),
    }
}

/// The `count` values of the input `spec`, an n × n matrix for `dd` and `spd`.
fn generate(spec: &str, count: usize, n: usize) -> Vec<f64> {
    if spec == "zero" {
        return vec![0.0; count];
    }
    if let Some(list) = spec.strip_prefix("bits:") {
        let out: Vec<f64> = list
            .split(',')
            .filter(|h| !h.is_empty())
            .map(|h| float::from_bits(u64::from_str_radix(h, 16).unwrap()))
            .collect();
        assert_eq!(out.len(), count, "{spec}");
        return out;
    }
    let (kind, seed) = spec.split_once(':').unwrap();
    let mut draws = Draws::new(seed.parse().unwrap());
    match kind {
        "u" | "w" | "s" => (0..count).map(|_| value(kind, &mut draws)).collect(),
        "dd" | "spd" => {
            let us: Vec<f64> = (0..count).map(|_| uniform(draws.next())).collect();
            let size = n as f64;
            let mut out = vec![0.0; count];
            for i in 0..n {
                for j in 0..n {
                    let u = us[i * n + j];
                    if kind == "dd" {
                        out[i * n + j] = if i == j { float::add(u, size) } else { u };
                    } else if i == j {
                        out[i * n + j] = float::add(size, float::abs(u));
                    } else if i > j {
                        out[i * n + j] = u;
                        out[j * n + i] = u;
                    }
                }
            }
            out
        }
        _ => panic!("no generator `{kind}`"),
    }
}

fn fnv(values: &[f64]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for x in values {
        for byte in float::to_bits(*x).to_le_bytes() {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    h
}

fn hex(text: &str) -> u64 {
    u64::from_str_radix(text, 16).unwrap()
}

/// One case: its kernel, its fields, and its line for the messages.
struct Case<'a> {
    kernel: &'a str,
    fields: BTreeMap<&'a str, &'a str>,
    positional: Vec<&'a str>,
    line: &'a str,
}

impl<'a> Case<'a> {
    fn parse(line: &'a str) -> Case<'a> {
        let mut words = line.split_whitespace();
        let kernel = words.next().unwrap();
        let mut fields = BTreeMap::new();
        let mut positional = Vec::new();
        for word in words {
            match word.split_once('=') {
                Some((key, value)) => {
                    fields.insert(key, value);
                }
                None => positional.push(word),
            }
        }
        Case {
            kernel,
            fields,
            positional,
            line,
        }
    }

    fn size(&self, key: &str) -> usize {
        self.fields[key].parse().unwrap()
    }

    fn input(&self, key: &str, count: usize, n: usize) -> Vec<f64> {
        let mut values = generate(self.fields[key], count, n);
        if let Some(patch) = self.fields.get("patch")
            && key == self.first_input()
        {
            for entry in patch.split(',') {
                let (position, bits) = entry.split_once(':').unwrap();
                values[position.parse::<usize>().unwrap()] = float::from_bits(hex(bits));
            }
        }
        values
    }

    /// The input that a patch applies to: the first one of the kernel.
    fn first_input(&self) -> &'static str {
        match self.kernel {
            "sum" | "dot" | "norm2" => "x",
            "lower" | "upper" => "t",
            _ => "a",
        }
    }

    /// Checks the digest of all the inputs, in their order.
    fn inputs_are(&self, inputs: &[&[f64]]) {
        let all: Vec<f64> = inputs.iter().flat_map(|xs| xs.iter().copied()).collect();
        assert_eq!(
            fnv(&all),
            hex(self.fields["in"]),
            "the generated inputs differ from the corpus's: {}",
            self.line
        );
    }

    /// Checks `got` against the expected outputs; `what` names the implementation.
    fn outputs_are(&self, got: &[f64], what: &str) {
        if let Some(listed) = self.fields.get("out") {
            let want: Vec<u64> = listed
                .split(',')
                .filter(|h| !h.is_empty())
                .map(hex)
                .collect();
            let got: Vec<u64> = got.iter().map(|x| float::to_bits(*x)).collect();
            assert_eq!(got, want, "{what}: {}", self.line);
        } else {
            let (digest, count) = self.fields["digest"].split_once(':').unwrap();
            assert_eq!(
                got.len(),
                count.parse::<usize>().unwrap(),
                "{what}: {}",
                self.line
            );
            assert_eq!(fnv(got), hex(digest), "{what}: {}", self.line);
        }
    }
}

fn run(case: &Case<'_>) {
    match case.kernel {
        "sum" => {
            let n = case.size("n");
            let x = case.input("x", n, 0);
            case.inputs_are(&[&x]);
            case.outputs_are(&[dense::sum_f64(x.iter().copied())], "reference");
            case.outputs_are(&[blocked::sum(&x)], "blocked");
        }
        "dot" => {
            let n = case.size("n");
            let (x, y) = (case.input("x", n, 0), case.input("y", n, 0));
            case.inputs_are(&[&x, &y]);
            let pairs = || x.iter().copied().zip(y.iter().copied());
            case.outputs_are(&[dense::dot_f64(pairs())], "reference");
            case.outputs_are(&[blocked::dot(&x, &y)], "blocked");
            case.outputs_are(&[blocked::dot_pairs(0.0, pairs())], "blocked pairs");
        }
        "norm2" => {
            let n = case.size("n");
            let x = case.input("x", n, 0);
            case.inputs_are(&[&x]);
            case.outputs_are(&[dense::norm2_f64(x.iter().copied())], "reference");
            case.outputs_are(&[blocked::norm2(x.iter().copied())], "blocked");
        }
        "multiply" => {
            let (m, k, n) = (case.size("m"), case.size("k"), case.size("n"));
            let (a, b, c) = (
                case.input("a", m * k, 0),
                case.input("b", k * n, 0),
                case.input("c", m * n, 0),
            );
            if case.fields["c"] == "zero" {
                case.inputs_are(&[&a, &b]);
            } else {
                case.inputs_are(&[&a, &b, &c]);
            }
            let mut want = Vec::new();
            for i in 0..m {
                for j in 0..n {
                    let pairs = (0..k).map(|t| (a[i * k + t], b[t * n + j]));
                    want.push(float::canonical(dense::dot_from(c[i * n + j], pairs)));
                }
            }
            case.outputs_are(&want, "reference");
            let mut got = c.clone();
            blocked::add_product(&mut got, &a, &b, m, k, n);
            case.outputs_are(&got, "blocked");
        }
        "multiply_array" => {
            let (m, k) = (case.size("m"), case.size("k"));
            let (a, v) = (case.input("a", m * k, 0), case.input("v", k, 0));
            case.inputs_are(&[&a, &v]);
            let want: Vec<f64> = (0..m)
                .map(|i| dense::dot_f64((0..k).map(|t| (a[i * k + t], v[t]))))
                .collect();
            case.outputs_are(&want, "reference");
            let mut got = vec![0.0; m];
            blocked::multiply_array(&a, m, k, &v, &mut got);
            case.outputs_are(&got, "blocked");
        }
        "lu" => {
            let n = case.size("n");
            let a = case.input("a", n * n, n);
            case.inputs_are(&[&a]);
            let ok = case.fields["ok"] == "1";
            let pivots: Vec<usize> = case.fields["pivots"]
                .split(',')
                .filter(|p| !p.is_empty())
                .map(|p| p.parse().unwrap())
                .collect();
            for (what, run) in [
                (
                    "reference",
                    dense::lu_f64 as fn(&mut [f64], usize, &mut [usize]) -> bool,
                ),
                ("blocked", blocked::lu),
            ] {
                let mut f = a.clone();
                let mut p = vec![0; n];
                assert_eq!(run(&mut f, n, &mut p), ok, "{what}: {}", case.line);
                assert_eq!(p, pivots, "{what}: {}", case.line);
                case.outputs_are(&f, what);
            }
        }
        "lu_solve" => {
            let (n, r) = (case.size("n"), case.size("r"));
            let (a, b) = (case.input("a", n * n, n), case.input("b", n * r, 0));
            case.inputs_are(&[&a, &b]);
            let mut f = a.clone();
            let mut p = vec![0; n];
            assert!(dense::lu_f64(&mut f, n, &mut p));
            let mut x = b.clone();
            dense::lu_solve_f64(&f, &p, n, &mut x, r);
            case.outputs_are(&x, "reference");
            let mut g = a.clone();
            let mut q = vec![0; n];
            assert!(blocked::lu(&mut g, n, &mut q));
            let mut y = b.clone();
            blocked::lu_solve(&g, &q, n, &mut y, r);
            case.outputs_are(&y, "blocked");
        }
        "cholesky" => {
            let n = case.size("n");
            let a = case.input("a", n * n, n);
            case.inputs_are(&[&a]);
            let ok = case.fields["ok"] == "1";
            for (what, run) in [
                (
                    "reference",
                    dense::cholesky_f64 as fn(&mut [f64], usize) -> bool,
                ),
                ("blocked", blocked::cholesky),
            ] {
                let mut l = a.clone();
                assert_eq!(run(&mut l, n), ok, "{what}: {}", case.line);
                case.outputs_are(&l, what);
            }
        }
        "lower" | "upper" => {
            let (n, r) = (case.size("n"), case.size("r"));
            let unit = case.fields["unit"] == "1";
            let (t, b) = (case.input("t", n * n, n), case.input("b", n * r, 0));
            case.inputs_are(&[&t, &b]);
            let triangle = if case.kernel == "lower" {
                Triangle::Lower
            } else {
                Triangle::Upper
            };
            let mut x = b.clone();
            dense::solve_triangular_f64(&t, n, &mut x, r, triangle, unit);
            case.outputs_are(&x, "reference");
            let mut y = b.clone();
            blocked::solve_triangular(&t, n, &mut y, r, triangle, unit);
            case.outputs_are(&y, "blocked");
        }
        "fma" => {
            let [x, y, z] = [0, 1, 2].map(|k| f64::from_bits(hex(case.positional[k])));
            let want = hex(case.fields["out"]);
            assert_eq!(
                float::to_bits(float::fma(x, y, z)),
                want,
                "reference: {}",
                case.line
            );
            assert_eq!(
                float::to_bits(fma::emulated(x, y, z)),
                want,
                "emulated: {}",
                case.line
            );
            assert_eq!(
                float::to_bits(fma::baseline(x, y, z)),
                want,
                "baseline: {}",
                case.line
            );
        }
        "ldexp" => {
            let y = f64::from_bits(hex(case.positional[0]));
            let k: i64 = case.positional[1].parse().unwrap();
            let want = hex(case.fields["out"]);
            assert_eq!(float::to_bits(float::ldexp(y, k)), want, "{}", case.line);
        }
        other => panic!("no kernel `{other}` in the corpus"),
    }
}

#[test]
fn every_kernel_reproduces_the_corpus_bit_for_bit() {
    let mut seen = BTreeMap::new();
    for line in CORPUS.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let case = Case::parse(line);
        run(&case);
        *seen.entry(case.kernel).or_insert(0_usize) += 1;
    }
    // Every kernel has its cases: a truncated corpus fails here.
    for kernel in [
        "sum",
        "dot",
        "norm2",
        "multiply",
        "multiply_array",
        "lu",
        "lu_solve",
        "cholesky",
        "lower",
        "upper",
        "fma",
        "ldexp",
    ] {
        assert!(
            seen.get(kernel).copied().unwrap_or(0) >= 5,
            "{kernel}: {seen:?}"
        );
    }
}
