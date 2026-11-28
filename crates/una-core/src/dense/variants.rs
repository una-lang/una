//! The target variants of the dense kernels (ADR 0120 rules 53, 56 and 57): the blocked
//! kernels of [`super::blocked`] compiled once per target, each with exactly the target
//! features its detection checks, and the table that a run calls them through.
//!
//! - [`baseline`]: no feature beyond the target's baseline, with [`super::fma::baseline`]:
//!   the `fmadd` instruction on AArch64, whose baseline has it, and the exact emulation of
//!   [`super::fma::emulated`] on x86-64 without FMA3.
//! - [`x86_64_v3`] and [`x86_64_v4`], on x86-64 only: the same source under
//!   `#[target_feature]`, whose `fma` is the processor's `vfmadd` instruction, by
//!   `core::arch::x86_64::_mm_fmadd_sd` on a scalar lane, and never a call.
//!
//! Every variant computes every element by the chain of its reference (rule 44): the variants
//! differ only in the instructions the compiler may choose, never in an operation, an operand
//! or an order, and Rust contracts no `a * b + c` and has no fast-math flag, so every variant
//! gives the reference's bits (rule 53). Each kernel's complexity is its blocked kernel's.
//!
//! Declaring a variant is safe Rust: a safe `#[target_feature]` function. Calling one from code
//! without its features is not, so this crate never does: `una-rt`'s `ffi.rs` holds the one
//! `unsafe` site, a macro that calls a variant only after the detection of exactly its
//! [`X86_64_V3_FEATURES`] or [`X86_64_V4_FEATURES`] succeeded (rule 56), and fills a
//! [`Kernels`] table with safe wrappers. The interpreter, which has no `unsafe`, runs the
//! table it is given, [`Kernels::BASELINE`] by default.
//!
//! **Exactly the features detected (rule 56).** Rust's `#[target_feature]` implies other
//! features (`avx` implies `sse4.2`, which implies `sse4.1`, `ssse3` and `sse3`; `avx512f`
//! implies `avx2`, `fma` and `f16c`), so each variant enables, and its detection checks, the
//! whole closure of its features under the toolchain's implications: x86-64-v3's `avx`, `avx2`
//! and `fma` with what they imply, and x86-64-v4's `avx512f`, `avx512vl`, `avx512dq` and
//! `avx512bw` with what they imply. `tests/variants_features.rs` asks the toolchain for that
//! closure and asserts that it equals [`X86_64_V3_FEATURES`] and [`X86_64_V4_FEATURES`], so
//! a processor or a virtual machine that hides an extension never runs an instruction it
//! lacks.

use super::Triangle;

/// One module of variants: `$name`, its target features, and its `fma` as an expression of
/// three `f64` operands. The features are both the `#[target_feature]` attributes of every
/// function of the module and the list its `detected` checks, from the same tokens, so the two
/// cannot differ (rule 56).
macro_rules! variant {
    (
        $(#[$attribute:meta])*
        $name:ident,
        $list:ident = [$($feature:tt),*],
        |$x:ident, $y:ident, $z:ident| $fma:expr
    ) => {
        #[doc = concat!(
            "The target features of the variant `", stringify!($name), "`, on every host, ",
            "so that the test of rule 56 reads them where the variant is not compiled."
        )]
        pub const $list: &[&str] = &[$($feature),*];

        $(#[$attribute])*
        #[allow(clippy::redundant_closure)]
        // Clippy reads a safe `#[target_feature]` function as an unsafe one and asks for a
        // `# Safety` section, on x86-64 only, where the variants have features (#2106). The
        // condition of a call, the variant's features detected first, is stated once in this
        // module's documentation above, and the one call site is `una-rt`'s `ffi.rs` (rule 56).
        #[allow(clippy::missing_safety_doc)]
        pub mod $name {
            #[allow(unused_imports)]
            use crate::dense::{Triangle, blocked, fma};
            #[allow(unused_imports)]
            use crate::float;

            /// The name of this variant, as the build report lists it.
            pub const NAME: &str = stringify!($name);

            /// The target features this variant enables, exactly those [`detected`] checks
            /// (ADR 0120 rule 56).
            pub const FEATURES: &[&str] = super::$list;

            /// Whether the processor has every feature of [`FEATURES`], by the standard
            /// library's safe detection; true for a variant with none. O(1) after the
            /// standard library's first detection, which it caches.
            pub fn detected() -> bool {
                true $(&& std::arch::is_x86_feature_detected!($feature))*
            }

            /// The `fma` of this variant, correctly rounded and canonical. O(1).
            $(#[target_feature(enable = $feature)])*
            #[inline]
            fn fused($x: f64, $y: f64, $z: f64) -> f64 {
                $fma
            }

            /// [`blocked::sum`] with this variant's features: the lanes of rule 45. O(n).
            ///
            /// # Safety
            ///
            /// The processor must have every feature of [`FEATURES`]: call it only where
            /// [`detected`] is true (ADR 0120 rule 56).
            $(#[target_feature(enable = $feature)])*
            pub fn sum(items: &[f64]) -> f64 {
                blocked::sum_lanes(items)
            }

            /// [`blocked::dot_from`] with this variant's `fma`: rule 46 from `start`. O(K).
            ///
            /// # Safety
            ///
            /// The processor must have every feature of [`FEATURES`]: call it only where
            /// [`detected`] is true (ADR 0120 rule 56).
            $(#[target_feature(enable = $feature)])*
            pub fn dot_from(start: f64, x: &[f64], y: &[f64]) -> f64 {
                blocked::dot_from_with(start, x, y, |a, b, c| fused(a, b, c))
            }

            /// [`blocked::dot_pairs`] with this variant's `fma`, over the pairs of `pairs`.
            /// O(K).
            ///
            /// # Safety
            ///
            /// The processor must have every feature of [`FEATURES`]: call it only where
            /// [`detected`] is true (ADR 0120 rule 56).
            $(#[target_feature(enable = $feature)])*
            pub fn dot_pairs(start: f64, pairs: &mut dyn Iterator<Item = (f64, f64)>) -> f64 {
                blocked::dot_pairs_with(start, pairs, |a, b, c| fused(a, b, c))
            }

            /// [`blocked::norm2`] with this variant's `fma` (rule 47): `scan` and `items` are
            /// two passes over the same elements. O(n).
            ///
            /// # Safety
            ///
            /// The processor must have every feature of [`FEATURES`]: call it only where
            /// [`detected`] is true (ADR 0120 rule 56).
            $(#[target_feature(enable = $feature)])*
            pub fn norm2(
                scan: &mut dyn Iterator<Item = f64>,
                items: &mut dyn Iterator<Item = f64>,
            ) -> f64 {
                blocked::norm2_with(scan, items, |a, b, c| fused(a, b, c))
            }

            /// [`blocked::add_product`] with this variant's `fma`: `C += A × B`. O(mkn).
            ///
            /// # Safety
            ///
            /// The processor must have every feature of [`FEATURES`]: call it only where
            /// [`detected`] is true (ADR 0120 rule 56).
            $(#[target_feature(enable = $feature)])*
            pub fn add_product(c: &mut [f64], a: &[f64], b: &[f64], m: usize, k: usize, n: usize) {
                blocked::add_product_with(c, a, b, m, k, n, |x, y, z| fused(x, y, z));
            }

            /// [`blocked::multiply_array`] with this variant's `fma`. O(mk).
            ///
            /// # Safety
            ///
            /// The processor must have every feature of [`FEATURES`]: call it only where
            /// [`detected`] is true (ADR 0120 rule 56).
            $(#[target_feature(enable = $feature)])*
            pub fn multiply_array(a: &[f64], m: usize, k: usize, v: &[f64], out: &mut [f64]) {
                blocked::multiply_array_with(a, m, k, v, out, |x, y, z| fused(x, y, z));
            }

            /// [`blocked::lu`] with this variant's `fma` (rule 48). O(n³).
            ///
            /// # Safety
            ///
            /// The processor must have every feature of [`FEATURES`]: call it only where
            /// [`detected`] is true (ADR 0120 rule 56).
            $(#[target_feature(enable = $feature)])*
            pub fn lu(a: &mut [f64], n: usize, pivots: &mut [usize]) -> bool {
                blocked::lu_with(a, n, pivots, |x, y, z| fused(x, y, z))
            }

            /// [`blocked::cholesky`] with this variant's `fma` (rule 49). O(n³).
            ///
            /// # Safety
            ///
            /// The processor must have every feature of [`FEATURES`]: call it only where
            /// [`detected`] is true (ADR 0120 rule 56).
            $(#[target_feature(enable = $feature)])*
            pub fn cholesky(a: &mut [f64], n: usize) -> bool {
                blocked::cholesky_with(a, n, |x, y, z| fused(x, y, z))
            }

            /// [`blocked::solve_triangular`] with this variant's `fma` (rule 50). O(n²r).
            ///
            /// # Safety
            ///
            /// The processor must have every feature of [`FEATURES`]: call it only where
            /// [`detected`] is true (ADR 0120 rule 56).
            $(#[target_feature(enable = $feature)])*
            pub fn solve_triangular(
                t: &[f64],
                n: usize,
                b: &mut [f64],
                r: usize,
                triangle: Triangle,
                unit_diagonal: bool,
            ) {
                blocked::solve_triangular_with(t, n, b, r, triangle, unit_diagonal, |x, y, z| {
                    fused(x, y, z)
                });
            }

            /// [`blocked::lu_solve`] with this variant's `fma` (rule 48). O(n²r).
            ///
            /// # Safety
            ///
            /// The processor must have every feature of [`FEATURES`]: call it only where
            /// [`detected`] is true (ADR 0120 rule 56).
            $(#[target_feature(enable = $feature)])*
            pub fn lu_solve(factors: &[f64], pivots: &[usize], n: usize, b: &mut [f64], r: usize) {
                blocked::lu_solve_with(factors, pivots, n, b, r, |x, y, z| fused(x, y, z));
            }
        }
    };
}

variant!(
    /// The kernels on the target's baseline, with no feature enabled: the `fma` of
    /// [`fma::baseline`], the `fmadd` instruction on AArch64 and the exact emulation on x86-64.
    /// Every processor of the target runs it, and the interpreter runs it by default.
    baseline,
    BASELINE_FEATURES = [],
    |x, y, z| fma::baseline(x, y, z)
);

variant!(
    /// The kernels for x86-64-v3 (Haswell, 2013, and later): `avx`, `avx2` and `fma`, with the
    /// SSE levels `avx` implies, and the `vfmadd` instruction as the `fma`.
    #[cfg(target_arch = "x86_64")]
    x86_64_v3,
    X86_64_V3_FEATURES = ["avx", "avx2", "fma", "sse3", "ssse3", "sse4.1", "sse4.2"],
    |x, y, z| {
        use core::arch::x86_64::{_mm_cvtsd_f64, _mm_fmadd_sd, _mm_set_sd};
        // The instruction on the low lane: x * y + z rounded once, as IEEE `fusedMultiplyAdd`.
        float::canonical(_mm_cvtsd_f64(_mm_fmadd_sd(
            _mm_set_sd(x),
            _mm_set_sd(y),
            _mm_set_sd(z),
        )))
    }
);

variant!(
    /// The kernels for x86-64-v4: x86-64-v3's features, `f16c`, which `avx512f` implies, and
    /// `avx512f`, `avx512vl`, `avx512dq` and `avx512bw`, with the `vfmadd` instruction as the
    /// `fma`.
    #[cfg(target_arch = "x86_64")]
    x86_64_v4,
    X86_64_V4_FEATURES = [
        "avx", "avx2", "fma", "sse3", "ssse3", "sse4.1", "sse4.2", "f16c", "avx512f", "avx512vl",
        "avx512dq", "avx512bw"
    ],
    |x, y, z| {
        use core::arch::x86_64::{_mm_cvtsd_f64, _mm_fmadd_sd, _mm_set_sd};
        // The instruction on the low lane: x * y + z rounded once, as IEEE `fusedMultiplyAdd`.
        float::canonical(_mm_cvtsd_f64(_mm_fmadd_sd(
            _mm_set_sd(x),
            _mm_set_sd(y),
            _mm_set_sd(z),
        )))
    }
);

/// The demangled module prefixes of the variant functions that may hold an FMA instruction:
/// the release gate of `scripts/check-fma.sh` fails on any `vfmadd`, `vfmsub`, `vfnmadd` or
/// `vfnmsub` in a symbol of an x86-64 artifact whose demangled name starts with none of them
/// (ADR 0120 Appendix A, the amendment of ADR 0106 item 3; ADR 0114 Appendix C rules 7 and
/// 8). The build report records it (`spec/13 §6`). The same on every host: it describes the
/// x86-64 artifacts, the only ones the gate scans.
pub const ALLOWLIST: &[&str] = &[
    "una_core::dense::variants::x86_64_v3",
    "una_core::dense::variants::x86_64_v4",
];

/// The names of the variants compiled for this target, in order of preference from last to
/// first: `baseline`, then `x86_64_v3` and `x86_64_v4` on x86-64.
#[cfg(target_arch = "x86_64")]
const NAMES: &[&str] = &[baseline::NAME, x86_64_v3::NAME, x86_64_v4::NAME];

/// The names of the variants compiled for this target: the baseline alone, away from x86-64.
#[cfg(not(target_arch = "x86_64"))]
const NAMES: &[&str] = &[baseline::NAME];

/// The names of the variants compiled for this target, which every native executable built
/// for it holds and chooses among at start (rules 56 and 57), as the build report lists them:
/// `baseline`, `x86_64_v3` and `x86_64_v4` on x86-64, `baseline` elsewhere. O(1).
pub fn variant_names() -> &'static [&'static str] {
    NAMES
}

/// The signature of [`super::blocked::add_product`]: `c += a × b` of an m × k and a k × n
/// matrix, row-major, with m, k and n.
pub type AddProduct = fn(&mut [f64], &[f64], &[f64], usize, usize, usize);

/// The kernels of one variant, as safe function pointers: the table a run calls every dense
/// kernel with an `fma` through. [`Kernels::BASELINE`] is built here; the tables of the
/// x86-64 variants are built by `una-rt`'s `ffi.rs`, after the detection of their features
/// (rule 56). Every table gives the same bits (rule 53).
#[derive(Clone, Copy)]
pub struct Kernels {
    /// The variant's name, one of [`variant_names`].
    pub name: &'static str,
    /// [`super::blocked::sum`].
    pub sum: fn(&[f64]) -> f64,
    /// [`super::blocked::dot_from`].
    pub dot_from: fn(f64, &[f64], &[f64]) -> f64,
    /// [`super::blocked::dot_pairs`], over a borrowed iterator.
    pub dot_pairs: fn(f64, &mut dyn Iterator<Item = (f64, f64)>) -> f64,
    /// [`super::blocked::norm2`], over two passes of the same elements.
    pub norm2: fn(&mut dyn Iterator<Item = f64>, &mut dyn Iterator<Item = f64>) -> f64,
    /// [`super::blocked::add_product`].
    pub add_product: AddProduct,
    /// [`super::blocked::multiply_array`].
    pub multiply_array: fn(&[f64], usize, usize, &[f64], &mut [f64]),
    /// [`super::blocked::lu`].
    pub lu: fn(&mut [f64], usize, &mut [usize]) -> bool,
    /// [`super::blocked::cholesky`].
    pub cholesky: fn(&mut [f64], usize) -> bool,
    /// [`super::blocked::solve_triangular`].
    pub solve_triangular: fn(&[f64], usize, &mut [f64], usize, Triangle, bool),
    /// [`super::blocked::lu_solve`].
    pub lu_solve: fn(&[f64], &[usize], usize, &mut [f64], usize),
}

impl Kernels {
    /// The kernels of [`baseline`], which every processor of the target runs.
    pub const BASELINE: Kernels = Kernels {
        name: baseline::NAME,
        sum: baseline::sum,
        dot_from: baseline::dot_from,
        dot_pairs: baseline::dot_pairs,
        norm2: baseline::norm2,
        add_product: baseline::add_product,
        multiply_array: baseline::multiply_array,
        lu: baseline::lu,
        cholesky: baseline::cholesky,
        solve_triangular: baseline::solve_triangular,
        lu_solve: baseline::lu_solve,
    };

    /// `x.dot(with: y)` by this table: [`Kernels::dot_from`] at `+0`. O(K).
    pub fn dot(&self, x: &[f64], y: &[f64]) -> f64 {
        (self.dot_from)(0.0, x, y)
    }

    /// The inner product of rule 46 over `pairs` from `start`, by this table's
    /// [`Kernels::dot_pairs`]. O(K).
    pub fn dot_of_pairs<I: IntoIterator<Item = (f64, f64)>>(&self, start: f64, pairs: I) -> f64 {
        (self.dot_pairs)(start, &mut pairs.into_iter())
    }

    /// `norm2` of `items` by this table's [`Kernels::norm2`], which reads a clone of `items`
    /// first and then `items`. O(n).
    pub fn norm2_of<I: Iterator<Item = f64> + Clone>(&self, items: I) -> f64 {
        let mut scan = items.clone();
        let mut items = items;
        (self.norm2)(&mut scan, &mut items)
    }
}

impl std::fmt::Debug for Kernels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kernels").field("name", &self.name).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_allowlist_names_the_modules_of_the_x86_64_variants() {
        // `module_path!()` here is the tests module's; the variants are its parent's.
        let parent = module_path!().trim_end_matches("::tests");
        assert_eq!(parent, "una_core::dense::variants");
        assert_eq!(
            ALLOWLIST,
            &[
                "una_core::dense::variants::x86_64_v3",
                "una_core::dense::variants::x86_64_v4"
            ]
        );
        #[cfg(target_arch = "x86_64")]
        {
            assert_eq!(
                ALLOWLIST,
                &[
                    format!("una_core::dense::variants::{}", x86_64_v3::NAME),
                    format!("una_core::dense::variants::{}", x86_64_v4::NAME),
                ]
            );
        }
    }

    #[test]
    fn the_names_of_the_variants_are_their_modules() {
        assert_eq!(variant_names().first(), Some(&"baseline"));
        assert_eq!(Kernels::BASELINE.name, "baseline");
        #[cfg(target_arch = "x86_64")]
        assert_eq!(variant_names(), &["baseline", "x86_64_v3", "x86_64_v4"]);
        #[cfg(not(target_arch = "x86_64"))]
        assert_eq!(variant_names(), &["baseline"]);
    }

    #[test]
    fn the_baseline_table_is_the_blocked_kernels() {
        let x: Vec<f64> = (0..300).map(|k| f64::from(k) * 0.37 - 41.0).collect();
        let y: Vec<f64> = (0..300).map(|k| 1.0 / (f64::from(k) + 0.5)).collect();
        let k = Kernels::BASELINE;
        let bits = |v: f64| v.to_bits();
        assert_eq!(bits((k.sum)(&x)), bits(super::super::blocked::sum(&x)));
        assert_eq!(
            bits(k.dot(&x, &y)),
            bits(super::super::blocked::dot(&x, &y))
        );
        let pairs = || x.iter().copied().zip(y.iter().copied());
        assert_eq!(
            bits(k.dot_of_pairs(1.5, pairs())),
            bits(super::super::blocked::dot_pairs(1.5, pairs()))
        );
        assert_eq!(
            bits(k.norm2_of(x.iter().copied())),
            bits(super::super::blocked::norm2(x.iter().copied()))
        );
    }
}
