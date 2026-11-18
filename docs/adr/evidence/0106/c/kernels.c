/* The kernels as a C-emitting backend would write them: every integer operation checked
   (the overflow path stands in for promotion to a big integer), -fwrapv so that no check
   is removed as undefined behavior. K3 is not here: it measures the runtime, not the backend. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
static void slow(void) { fputs("overflow: promote\n", stderr); exit(3); }
static int64_t add(int64_t a, int64_t b) { int64_t r; if (__builtin_add_overflow(a, b, &r)) slow(); return r; }
static int64_t sub(int64_t a, int64_t b) { int64_t r; if (__builtin_sub_overflow(a, b, &r)) slow(); return r; }
static int64_t mul(int64_t a, int64_t b) { int64_t r; if (__builtin_mul_overflow(a, b, &r)) slow(); return r; }
static int64_t emod(int64_t a, int64_t m) { int64_t r = a % m; return r < 0 ? r + m : r; }
static int64_t fib(int64_t n) { return n < 2 ? n : add(fib(sub(n, 1)), fib(sub(n, 2))); }
static int64_t mix(int64_t n) { int64_t h = 7; for (int64_t i = 0; i < n; i++) h = emod(add(mul(h, 31), i), 1000000007); return h; }
int main(int argc, char **argv) { if (argc > 1 && argv[1][0] == 'm') printf("%lld\n", (long long)mix(50000000)); else printf("%lld\n", (long long)fib(32)); return 0; }
