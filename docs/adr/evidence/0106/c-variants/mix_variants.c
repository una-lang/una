/* K2 variants, to explain the Rust-vs-C gap of ADR 0106: the same checked loop written several
   ways. VARIANT selects one at compile time; every variant must print the same value. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
static void slow(void) { fputs("overflow: promote\n", stderr); exit(3); }
static inline int64_t add(int64_t a, int64_t b) { int64_t r; if (__builtin_add_overflow(a, b, &r)) slow(); return r; }
static inline int64_t mul(int64_t a, int64_t b) { int64_t r; if (__builtin_mul_overflow(a, b, &r)) slow(); return r; }
#if VARIANT == 1
/* As in the measured kit: Euclidean mod with a branch. */
static inline int64_t emod(int64_t a, int64_t m) { int64_t r = a % m; return r < 0 ? r + m : r; }
#elif VARIANT == 2
/* As Rust's rem_euclid for a positive divisor: branch-free adjustment. */
static inline int64_t emod(int64_t a, int64_t m) { int64_t r = a % m; return r + (m & (r >> 63)); }
#elif VARIANT == 3
/* The divisor known positive at the call: the backend states it with __builtin_assume. */
static inline int64_t emod(int64_t a, int64_t m) { __builtin_assume(m > 0); int64_t r = a % m; return r < 0 ? r + m : r; }
#elif VARIANT == 4
/* The dividend known non-negative, as a verified range would let the backend state it: the
   remainder is taken unsigned, as LLVM does for Rust's rem_euclid when it can prove the sign. */
static inline int64_t emod(int64_t a, int64_t m) { __builtin_assume(a >= 0 && m > 0); return (int64_t)((uint64_t)a % (uint64_t)m); }
#endif
static int64_t mix(int64_t n) { int64_t h = 7; for (int64_t i = 0; i < n; i++) h = emod(add(mul(h, 31), i), 1000000007); return h; }
int main(void) { printf("%lld\n", (long long)mix(50000000)); return 0; }
